//! Bounded named data streams, captured with the base file and directory pinned.
use crate::runtime::cache::directory::Directory;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};

const MAX_STREAMS: usize = 64;
const MAX_BYTES: usize = 1024 * 1024;
const INFO_BYTES: usize = 64 * 1024;
type Streams = BTreeMap<String, Vec<u8>>;

fn valid_name(name: &str) -> bool {
    name.strip_prefix(':')
        .and_then(|s| s.strip_suffix(":$DATA"))
        .is_some_and(|s| !s.is_empty() && s.len() <= 1024 && !s.contains(['\0', ':', '/', '\\']))
}

pub(super) fn validate(streams: &Streams) -> Result<(), String> {
    if streams.len() > MAX_STREAMS
        || streams.keys().any(|s| !valid_name(s))
        || streams.values().map(Vec::len).sum::<usize>() > MAX_BYTES
    {
        return Err("invalid or oversized transaction named streams".into());
    }
    Ok(())
}

fn enumerate(file: &File) -> Result<BTreeMap<String, usize>, String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut std::ffi::c_void,
            class: i32,
            buffer: *mut std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    // FILE_STREAM_INFO requires eight-byte alignment, including subsequent entries.
    let mut buffer = vec![0u64; INFO_BYTES / 8];
    // SAFETY: live file handle and aligned writable buffer of the declared size.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            7,
            buffer.as_mut_ptr().cast(),
            INFO_BYTES as u32,
        )
    };
    if result == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(38) {
            // ERROR_HANDLE_EOF: no streams
            return Ok(BTreeMap::new());
        }
        return Err(format!(
            "cannot enumerate bounded transaction streams: {error}"
        ));
    }
    // SAFETY: initialized u64 storage can be viewed as bytes for bounds-checked parsing.
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), INFO_BYTES) };
    parse(bytes)
}

fn parse(bytes: &[u8]) -> Result<BTreeMap<String, usize>, String> {
    let mut offset = 0;
    let mut streams = BTreeMap::new();
    let mut total = 0usize;
    loop {
        let header = bytes
            .get(offset..offset + 24)
            .ok_or("invalid stream header")?;
        let next = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(header[8..16].try_into().unwrap());
        if length == 0 || length % 2 != 0 || length > 2048 {
            return Err("invalid stream name length".into());
        }
        let end = offset + 24 + length;
        let name = bytes
            .get(offset + 24..end)
            .ok_or("invalid stream name range")?;
        let utf16: Vec<_> = name
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let name = String::from_utf16(&utf16).map_err(|_| "unsupported stream name encoding")?;
        if name != "::$DATA" {
            if !valid_name(&name) || size > MAX_BYTES as u64 {
                return Err("unsupported or oversized transaction stream".into());
            }
            total += size as usize;
            if total > MAX_BYTES
                || streams.insert(name, size as usize).is_some()
                || streams.len() > MAX_STREAMS
            {
                return Err("transaction streams exceed limit or repeat".into());
            }
        }
        if next == 0 {
            break;
        }
        if next % 8 != 0 || next < 24 + length || next > bytes.len() - offset {
            return Err("invalid stream entry offset".into());
        }
        offset += next;
    }
    Ok(streams)
}

pub(super) fn capture_for_transaction(
    dir: &Directory,
    name: &str,
    base: &File,
    alias_handle: bool,
) -> Result<Streams, String> {
    let descriptors = enumerate(base)?;
    let base_id = super::files::identity(base, &base.metadata().map_err(|e| e.to_string())?)?;
    let mut streams = Streams::new();
    // Keep all stream handles open until capture ends; deny writes/deletion to each.
    let mut pins = Vec::new();
    for (stream, size) in &descriptors {
        dir.check_current().map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .read(true)
            // An alias-publication base handle holds DELETE access while denying
            // deletion itself. Stream readers must share that owned access.
            .share_mode(if alias_handle { 1 | 4 } else { 1 })
            .custom_flags(0x00200000)
            .open(dir.path.join(format!("{name}{stream}")))
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if super::files::identity(&file, &metadata)? != base_id || metadata.len() != *size as u64 {
            return Err("conflict: named stream changed during capture".into());
        }
        let mut content = Vec::new();
        (&mut file)
            .take(*size as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|e| e.to_string())?;
        if content.len() != *size {
            return Err("conflict: named stream size changed".into());
        }
        streams.insert(stream.clone(), content);
        pins.push(file);
    }
    if enumerate(base)? != descriptors {
        return Err("conflict: named stream list changed during capture".into());
    }
    dir.check_current().map_err(|e| e.to_string())?;
    Ok(streams)
}

pub(super) fn restore(
    dir: &Directory,
    name: &str,
    base: &File,
    streams: &Streams,
) -> Result<(), String> {
    validate(streams)?;
    let base_id = super::files::identity(base, &base.metadata().map_err(|e| e.to_string())?)?;
    for (stream, bytes) in streams {
        dir.check_current().map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            // The pinned private base handle owns DELETE access to clear its
            // generated short name. Stream sharing must admit that existing
            // access; the base itself still denies deletion by other handles.
            .share_mode(1 | 4)
            .custom_flags(0x00200000)
            .open(dir.path.join(format!("{name}{stream}")))
            .map_err(|e| e.to_string())?;
        if super::files::identity(&file, &file.metadata().map_err(|e| e.to_string())?)? != base_id {
            return Err("conflict: staged stream identity changed".into());
        }
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64) -> Vec<u8> {
        let encoded: Vec<_> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut bytes = vec![0; 24];
        bytes[4..8].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
        bytes[8..16].copy_from_slice(&size.to_le_bytes());
        bytes.extend(encoded);
        bytes
    }

    #[test]
    fn transaction_stream_parser_rejects_malformed_or_unbounded_metadata() {
        assert!(parse(&entry("::$DATA", 123)).unwrap().is_empty());
        assert_eq!(parse(&entry(":meta:$DATA", 9)).unwrap()[":meta:$DATA"], 9);
        for name in [
            ":../other:$DATA",
            ":a\\b:$DATA",
            ":a:b:$DATA",
            ":x:$INDEX_ALLOCATION",
            ":\0:$DATA",
        ] {
            assert!(parse(&entry(name, 0)).is_err(), "{name:?}");
        }
        assert!(parse(&entry(":meta:$DATA", MAX_BYTES as u64 + 1)).is_err());
        let mut bytes = entry(":meta:$DATA", 0);
        bytes[..4].copy_from_slice(&8u32.to_le_bytes());
        assert!(parse(&bytes).is_err());
        bytes[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&bytes).is_err());
        assert!(parse(&[0; 23]).is_err());
        let mut invalid = Streams::new();
        invalid.insert(":../escape:$DATA".into(), Vec::new());
        assert!(validate(&invalid).is_err());
    }
}
