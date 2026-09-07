//! Foreground scan interruption; handlers only set an atomic flag.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct Interrupt {
    flag: Arc<AtomicBool>,
    #[cfg(windows)]
    handler: unsafe extern "system" fn(u32) -> i32,
    #[cfg(unix)]
    id: signal_hook::SigId,
}
impl Interrupt {
    pub fn install() -> Result<Self, String> {
        #[cfg(windows)]
        {
            static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
            let flag = FLAG
                .get_or_init(|| Arc::new(AtomicBool::new(false)))
                .clone();
            flag.store(false, Ordering::Release);
            unsafe extern "system" fn handler(event: u32) -> i32 {
                if event <= 1 {
                    if let Some(flag) = FLAG.get() {
                        flag.store(true, Ordering::Release);
                    }
                    1
                } else {
                    0
                }
            }
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                return Err("cannot register security scan interruption".into());
            }
            Ok(Self { flag, handler })
        }
        #[cfg(unix)]
        {
            let flag = Arc::new(AtomicBool::new(false));
            let id = signal_hook::flag::register(signal_hook::consts::SIGINT, flag.clone())
                .map_err(|_| "cannot register security scan interruption")?;
            Ok(Self { flag, id })
        }
        #[cfg(not(any(windows, unix)))]
        {
            Err("scan interruption unsupported".into())
        }
    }
    pub fn requested(&self) -> bool {
        if self.flag.load(Ordering::Acquire) {
            return true;
        }
        std::env::var_os("PI_SECURITY_SCAN_INTERRUPT")
            .is_some_and(|path| std::path::Path::new(&path).exists())
    }
}
#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
impl Drop for Interrupt {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            unsafe {
                SetConsoleCtrlHandler(Some(self.handler), 0);
            }
        }
        #[cfg(unix)]
        {
            signal_hook::low_level::unregister(self.id);
        }
    }
}
