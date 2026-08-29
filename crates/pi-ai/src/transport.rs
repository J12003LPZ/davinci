//! Codex / OpenAI Responses transports: `sse`, `websocket`, `websocket-cached`, `auto`.
//! Live sockets are not opened in tests; decisions and cache keys match TypeScript.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Sse,
    Websocket,
    WebsocketCached,
    Auto,
}

impl Transport {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sse" => Some(Self::Sse),
            "websocket" => Some(Self::Websocket),
            "websocket-cached" => Some(Self::WebsocketCached),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sse => "sse",
            Self::Websocket => "websocket",
            Self::WebsocketCached => "websocket-cached",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportDecision {
    pub transport: Transport,
    pub websocket_failures: u32,
    pub websocket_fallback_active: bool,
    pub cache_key: Option<String>,
}

#[derive(Debug)]
pub struct CodexWebsocketCache {
    entries: HashMap<String, CachedSocket>,
    fallbacks: HashMap<String, u32>,
}

impl Default for CodexWebsocketCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexWebsocketCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            fallbacks: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedSocket {
    account: String,
    session_id: String,
}

impl CodexWebsocketCache {
    pub fn cache_key(account: &str, session_id: &str) -> String {
        format!("{account}::{session_id}")
    }

    pub fn resolve(
        &mut self,
        requested: Transport,
        account: &str,
        session_id: &str,
        websocket_open: bool,
    ) -> TransportDecision {
        if requested == Transport::Sse {
            return TransportDecision {
                transport: Transport::Sse,
                websocket_failures: *self.fallbacks.get(session_id).unwrap_or(&0),
                websocket_fallback_active: self.fallbacks.contains_key(session_id),
                cache_key: None,
            };
        }
        if self.fallbacks.contains_key(session_id) && requested != Transport::Websocket {
            return TransportDecision {
                transport: Transport::Sse,
                websocket_failures: *self.fallbacks.get(session_id).unwrap_or(&0),
                websocket_fallback_active: true,
                cache_key: None,
            };
        }
        if !websocket_open {
            let failures = self.fallbacks.entry(session_id.to_string()).or_insert(0);
            *failures += 1;
            return TransportDecision {
                transport: Transport::Sse,
                websocket_failures: *failures,
                websocket_fallback_active: true,
                cache_key: None,
            };
        }
        let key = Self::cache_key(account, session_id);
        if requested == Transport::WebsocketCached {
            self.entries.insert(
                key.clone(),
                CachedSocket {
                    account: account.to_string(),
                    session_id: session_id.to_string(),
                },
            );
        }
        TransportDecision {
            transport: if requested == Transport::Auto {
                Transport::WebsocketCached
            } else {
                requested
            },
            websocket_failures: 0,
            websocket_fallback_active: false,
            cache_key: Some(key),
        }
    }

    pub fn get(&self, account: &str, session_id: &str) -> bool {
        self.entries
            .get(&Self::cache_key(account, session_id))
            .is_some_and(|entry| entry.account == account && entry.session_id == session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cached_websockets_to_account() {
        let mut cache = CodexWebsocketCache::default();
        let a = cache.resolve(Transport::WebsocketCached, "acct-a", "shared", true);
        let b = cache.resolve(Transport::WebsocketCached, "acct-b", "shared", true);
        assert_ne!(a.cache_key, b.cache_key);
        assert!(cache.get("acct-a", "shared"));
        assert!(cache.get("acct-b", "shared"));
        assert!(!cache.get("acct-a", "other"));
    }

    #[test]
    fn falls_back_to_sse_when_websocket_does_not_open() {
        let mut cache = CodexWebsocketCache::default();
        let decision = cache.resolve(Transport::WebsocketCached, "acct", "s1", false);
        assert_eq!(decision.transport, Transport::Sse);
        assert!(decision.websocket_fallback_active);
        assert_eq!(decision.websocket_failures, 1);
        let again = cache.resolve(Transport::WebsocketCached, "acct", "s1", true);
        assert_eq!(again.transport, Transport::Sse);
        assert!(again.websocket_fallback_active);
    }
}
