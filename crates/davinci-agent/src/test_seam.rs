//! Test-only seam: lets a test put a counting, outgrowing reader in place of the
//! file a bounded consumer is about to read, so the consumer's own bound (not a
//! helper's) is what the test measures.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;

type Factory = Box<dyn Fn(u64) -> Box<dyn Read>>;

thread_local! {
    static READERS: RefCell<HashMap<&'static str, Factory>> = RefCell::new(HashMap::new());
}

/// Install a reader factory for `consumer` on this thread. It receives the
/// declared size the consumer saw.
pub(crate) fn install(consumer: &'static str, factory: impl Fn(u64) -> Box<dyn Read> + 'static) {
    READERS.with(|readers| readers.borrow_mut().insert(consumer, Box::new(factory)));
}

pub(crate) fn clear(consumer: &'static str) {
    READERS.with(|readers| readers.borrow_mut().remove(consumer));
}

/// The replacement reader for `consumer`, if a test installed one.
pub(crate) fn reader(consumer: &'static str, declared: u64) -> Option<Box<dyn Read>> {
    READERS.with(|readers| {
        readers
            .borrow()
            .get(consumer)
            .map(|factory| factory(declared))
    })
}

/// A source that yields `total` bytes whatever size was declared, counting what
/// the consumer pulled.
pub(crate) struct Outgrowing {
    pub left: u64,
    pub pulled: std::rc::Rc<std::cell::Cell<u64>>,
}

impl Read for Outgrowing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (buf.len() as u64).min(self.left) as usize;
        buf[..n].fill(b'x');
        self.left -= n as u64;
        self.pulled.set(self.pulled.get() + n as u64);
        Ok(n)
    }
}
