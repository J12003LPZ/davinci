//! User-only local voice setup; dispatched before providers and extensions.
use davinci_voice::catalog;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub mouse: bool,
    pub model: String,
    pub language: String,
    pub input_device: Option<String>,
    pub backend: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            mouse: true,
            model: "base".into(),
            language: "auto".into(),
            input_device: None,
            backend: "cpu".into(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, String> {
        Self::from_dir(&crate::default_agent_dir())
    }
    fn from_dir(dir: &Path) -> Result<Self, String> {
        let raw = match fs::read_to_string(dir.join("settings.json")) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err("Cannot read user voice settings".into()),
        };
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "Invalid user settings JSON")?;
        let config: Self = match value.get("voice") {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|_| "Invalid user voice settings")?
            }
            None => Self::default(),
        };
        if catalog::find(&config.model).is_none() || config.backend != "cpu" {
            return Err("Voice requires an approved tiny/base/small model and cpu backend".into());
        }
        let languages = "auto en zh de es ru ko fr ja pt tr pl ca nl ar sv it id hi fi vi he uk el ms cs ro da hu ta no th ur hr bg lt la mi ml cy sk te fa lv bn sr az sl kn et mk br eu is hy ne mn bs kk sq sw gl mr pa si km sn yo so af oc ka be tg sd gu am yi lo uz fo ht ps tk nn mt sa lb my bo tl mg as tt haw ln ha ba jw su yue";
        if !languages.split_whitespace().any(|x| x == config.language) {
            return Err("Invalid voice language; use auto or a Whisper language code".into());
        }
        Ok(config)
    }
}

pub fn model_path(id: &str) -> Result<PathBuf, String> {
    let model = catalog::find(id).ok_or("Unknown speech model")?;
    Ok(crate::default_agent_dir()
        .join("voice/models")
        .join(id)
        .join(format!("{}.bin", model.sha256)))
}

pub fn helper_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "Cannot locate this installation")?;
    Ok(exe.with_file_name(if cfg!(windows) {
        "davinci-voice-worker.exe"
    } else {
        "davinci-voice-worker"
    }))
}

fn install_reader(
    model: &catalog::Model,
    reader: &mut dyn Read,
    destination: &Path,
) -> Result<(), String> {
    install_reader_with(
        model,
        reader,
        destination,
        &std::sync::atomic::AtomicBool::new(false),
        &|_| {},
    )
}

fn install_reader_with(
    model: &catalog::Model,
    reader: &mut dyn Read,
    destination: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    let dir = destination.parent().ok_or("Invalid model destination")?;
    fs::create_dir_all(dir).map_err(|_| "Cannot create voice model directory")?;
    let lock_path = dir.join("install.lock");
    let lock = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|_| {
            "Model install already running or stale install.lock; inspect before retrying"
        })?;
    let part = dir.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(|_| "Cannot create model download")?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err("Model setup cancelled".into());
            }
            let count = reader
                .read(&mut buffer)
                .map_err(|_| "Speech model transfer interrupted")?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or("Model size overflow")?;
            if total > model.bytes {
                return Err("Speech model exceeds approved size".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|_| "Cannot write speech model; check disk space")?;
            hash.update(&buffer[..count]);
            progress(total);
        }
        if total != model.bytes || format!("{:x}", hash.finalize()) != model.sha256 {
            return Err("Speech model checksum or size mismatch".into());
        }
        file.sync_all().map_err(|_| "Cannot flush speech model")?;
        drop(file);
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Model setup cancelled".into());
        }
        // Immutable identity names: never overwrite a previous file on Windows/Unix.
        if destination.exists() {
            model.read_verified(destination).map_err(|_| {
                "Existing model is corrupt; remove that file explicitly before retrying"
            })?;
        } else {
            fs::hard_link(&part, destination)
                .map_err(|_| "Cannot publish verified model atomically")?;
            #[cfg(unix)]
            File::open(dir)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "Cannot flush model directory")?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&part);
    drop(lock);
    let _ = fs::remove_file(lock_path);
    result
}

pub fn import(id: &str, source: &Path) -> Result<(), String> {
    let model = catalog::find(id).ok_or("Unknown speech model")?;
    let mut file = File::open(source).map_err(|_| "Cannot open speech model for import")?;
    install_reader(model, &mut file, &model_path(id)?)
}

pub fn download(id: &str) -> Result<(), String> {
    download_with(id, &std::sync::atomic::AtomicBool::new(false), &|_| {})
}

pub fn setup(
    id: &str,
    source: Option<&Path>,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    if let Some(path) = source {
        let model = catalog::find(id).ok_or("Unknown speech model")?;
        let mut file = File::open(path).map_err(|_| "Cannot open speech model for import")?;
        install_reader_with(model, &mut file, &model_path(id)?, cancel, progress)
    } else {
        download_with(id, cancel, progress)
    }
}

fn download_with(
    id: &str,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    if crate::tools_manager::is_offline_mode_enabled() {
        return Err("Offline mode: import an approved local model instead".into());
    }
    let model = catalog::find(id).ok_or("Unknown speech model")?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let mut url = model.url();
    for _ in 0..5 {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Model setup cancelled".into());
        }
        if !url.starts_with("https://") {
            return Err("Speech model redirect requires HTTPS".into());
        }
        let response = agent
            .get(&url)
            .call()
            .map_err(|_| "Speech model download failed")?;
        if (300..400).contains(&response.status()) {
            let location = response
                .header("Location")
                .ok_or("Missing model redirect")?;
            url = url::Url::parse(&url)
                .and_then(|base| base.join(location))
                .map_err(|_| "Invalid model redirect")?
                .to_string();
            continue;
        }
        if response.status() != 200 {
            return Err("Unexpected model download response".into());
        }
        return install_reader_with(
            model,
            &mut response.into_reader(),
            &model_path(id)?,
            cancel,
            progress,
        );
    }
    Err("Too many model download redirects".into())
}

pub fn cli(args: &[String]) -> Result<i32, String> {
    let args: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--offline")
        .collect();
    match args.as_slice() {
        ["status"] => {
            let config = Config::load()?;
            println!("Local CPU voice: {}\nModel: {}\nHelper installed: {}\nModel present: {}\nDictation is inserted at your current cursor. It never sends.", if config.enabled {"enabled"} else {"disabled"}, config.model,helper_path()?.is_file(),model_path(&config.model)?.is_file());
        }
        ["devices"] => {
            let mut command=std::process::Command::new(helper_path()?);
            command.env_clear().arg("--devices");
            for name in ["SystemRoot","WINDIR","TEMP","TMP","HOME","USER","XDG_RUNTIME_DIR","DBUS_SESSION_BUS_ADDRESS","PULSE_SERVER","DISPLAY"] {
                if let Some(value)=std::env::var_os(name) {command.env(name,value);}
            }
            let status = command.status().map_err(|_|"Matching voice helper is missing")?;
            if !status.success() { return Err("Cannot enumerate microphone devices".into()); }
        }
        ["model","list"] => for model in catalog::MODELS { println!("{}: {} bytes, MIT, {}",model.id,model.bytes,model.sha256); },
        ["model","install",id] => { download(id)?; println!("Model installed. Activate voice separately to record."); },
        ["model","import",id,path] => { import(id,Path::new(path))?; println!("Model imported. Activate voice separately to record."); },
        _ => return Err("Usage: davinci voice status|devices|model list|model install <tiny|base|small>|model import <id> <path>".into()),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_voice_defaults_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::from_dir(dir.path()).unwrap().model, "base");
        fs::write(
            dir.path().join("settings.json"),
            r#"{"unrelated":true,"voice":{"model":"tiny","mouse":false}}"#,
        )
        .unwrap();
        let config = Config::from_dir(dir.path()).unwrap();
        assert_eq!(config.model, "tiny");
        assert!(!config.mouse);
        fs::write(
            dir.path().join("settings.json"),
            r#"{"voice":{"backend":"cloud"}}"#,
        )
        .unwrap();
        assert!(Config::from_dir(dir.path()).is_err());
    }
    #[test]
    fn transfer_failure_never_publishes_or_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.bin");
        fs::write(&destination, b"previous").unwrap();
        assert!(install_reader(&catalog::MODELS[0], &mut &b"bad"[..], &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn verified_publication_cancellation_and_install_lock() {
        let model = catalog::Model {
            id: "fixture",
            file: "fixture.bin",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.bin");
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        assert!(
            install_reader_with(&model, &mut &b"abc"[..], &destination, &cancelled, &|_| {})
                .is_err()
        );
        assert!(!destination.exists());
        install_reader(&model, &mut &b"abc"[..], &destination).unwrap();
        assert_eq!(model.read_verified(&destination).unwrap(), b"abc");
        fs::write(dir.path().join("install.lock"), b"busy").unwrap();
        assert!(install_reader(&model, &mut &b"abc"[..], &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"abc");
    }
}
