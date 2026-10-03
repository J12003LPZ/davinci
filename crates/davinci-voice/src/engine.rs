//! Opaque native context; changing whisper parameter layouts remain in C++.
use crate::{catalog, normalize, protocol::VoiceError};
use std::{
    ffi::{c_char, c_void, CString},
    marker::PhantomData,
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;

extern "C" {
    fn dv_load(bytes: *mut c_void, size: usize) -> *mut c_void;
    fn dv_load_file(path: *const c_char) -> *mut c_void;
    fn dv_free(context: *mut c_void);
    fn dv_decode(
        context: *mut c_void,
        pcm: *const f32,
        samples: usize,
        language: *const c_char,
        threads: i32,
        abort: extern "C" fn(*mut c_void) -> bool,
        user: *mut c_void,
        text: *mut c_char,
        capacity: usize,
    ) -> i32;
}

pub struct Engine {
    context: *mut c_void,
    _owner_thread: PhantomData<Rc<()>>,
}

extern "C" fn cancelled(user: *mut c_void) -> bool {
    // The synchronous call borrows an AtomicBool that outlives inference.
    unsafe { (*user.cast::<AtomicBool>()).load(Ordering::Acquire) }
}

impl Engine {
    pub fn load(id: &str, path: &Path) -> Result<Self, VoiceError> {
        let model = catalog::find(id).ok_or(VoiceError::ModelMissing)?;
        let verified = model.verify_file(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VoiceError::ModelMissing
            } else {
                VoiceError::ModelCorrupt
            }
        })?;
        // Models live in the user-owned agent directory. Re-check size and
        // modification time immediately before whisper reopens the path so a
        // changed model is refused without keeping a second copy in memory.
        let metadata = std::fs::metadata(path).map_err(|_| VoiceError::ModelMissing)?;
        if !verified.matches(&metadata) {
            return Err(VoiceError::ModelCorrupt);
        }
        let path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| VoiceError::ModelCorrupt)?;
        let context = unsafe { dv_load_file(path.as_ptr()) };
        if context.is_null() {
            return Err(VoiceError::ModelCorrupt);
        }
        Ok(Self {
            context,
            _owner_thread: PhantomData,
        })
    }

    pub fn decode(
        &mut self,
        pcm: &[f32],
        language: &str,
        cancel: &AtomicBool,
    ) -> Result<String, VoiceError> {
        if pcm.len() < 4800 {
            return Err(VoiceError::NoSpeech);
        }
        let language = CString::new(language).map_err(|_| VoiceError::UnsupportedBackend)?;
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .saturating_sub(1)
            .clamp(1, 4);
        let mut text = Zeroizing::new(vec![0u8; normalize::MAX_TEXT_BYTES + 1]);
        let result = unsafe {
            dv_decode(
                self.context,
                pcm.as_ptr(),
                pcm.len(),
                language.as_ptr(),
                threads as i32,
                cancelled,
                (cancel as *const AtomicBool).cast_mut().cast(),
                text.as_mut_ptr().cast(),
                text.len(),
            )
        };
        if result != 0 {
            return Err(if result == 3 {
                VoiceError::TextTooLong
            } else {
                VoiceError::InferenceFailed
            });
        }
        text_from_decoder(&text)
    }
}

fn text_from_decoder(buffer: &[u8]) -> Result<String, VoiceError> {
    let end = buffer
        .iter()
        .position(|b| *b == 0)
        .ok_or(VoiceError::InferenceFailed)?;
    let raw = String::from_utf8_lossy(&buffer[..end]);
    normalize::normalize(&raw).map_err(|_| VoiceError::TextTooLong)
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { dv_free(self.context) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an explicitly provisioned approved model; never downloads"]
    fn approved_model_offline_recognition_and_fresh_state() {
        let path =
            std::env::var_os("DAVINCI_VOICE_TEST_MODEL").expect("set DAVINCI_VOICE_TEST_MODEL");
        let id = std::env::var("DAVINCI_VOICE_TEST_MODEL_ID").unwrap_or_else(|_| "base".into());
        let started = std::time::Instant::now();
        let mut engine = Engine::load(&id, Path::new(&path)).unwrap();
        eprintln!("{id} cold load: {:?}", started.elapsed());
        let pcm: Vec<f32> = include_bytes!("../fixtures/jfk.pcm")
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0)
            .collect();
        let cancel = AtomicBool::new(false);
        for language in ["en", "auto"] {
            let started = std::time::Instant::now();
            let text = engine.decode(&pcm, language, &cancel).unwrap();
            eprintln!("{id} 11-second fixture decode: {:?}", started.elapsed());
            let words: String = text
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphabetic() || c.is_whitespace())
                .collect();
            assert!(words.contains("ask not what your country can do for you"));
        }
        assert!(crate::audio::no_speech(&vec![0.0; 16000]));
        cancel.store(true, Ordering::Release);
        assert!(engine.decode(&pcm, "en", &cancel).is_err());
    }

    #[test]
    fn invalid_utf8_from_the_decoder_is_decoded_lossily() {
        let bytes = b"caf\xc3 ok\0".to_vec();
        assert_eq!(text_from_decoder(&bytes).unwrap(), "caf\u{fffd} ok");
    }

    #[test]
    fn malformed_header_and_short_pcm_are_rejected() {
        let mut bytes = [0u8; 32];
        assert!(unsafe { dv_load(bytes.as_mut_ptr().cast(), bytes.len()) }.is_null());
        assert_eq!(
            unsafe {
                dv_decode(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    1,
                    cancelled,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            },
            2
        );
    }
}
