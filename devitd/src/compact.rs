//! Message compression for DevIt orchestration daemon
//!
//! Provides compact JSON format to reduce token usage by 60-80%.

use super::Msg;
use serde::{Deserialize, Serialize};

/// Compact message format with shortened field names
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MsgC {
    pub t: String,            // msg_type
    pub i: String,            // msg_id
    pub f: String,            // from
    pub o: String,            // to
    pub s: u64,               // ts
    pub n: String,            // nonce
    pub h: String,            // hmac
    pub p: serde_json::Value, // payload
}

/// Convert standard message to compact format
pub fn to_compact(m: &Msg) -> MsgC {
    MsgC {
        t: m.msg_type.clone(),
        i: m.msg_id.clone(),
        f: m.from.clone(),
        o: m.to.clone(),
        s: m.ts,
        n: m.nonce.clone(),
        h: m.hmac.clone(),
        p: m.payload.clone(),
    }
}

/// Convert compact message to standard format
pub fn from_compact(c: &MsgC) -> Msg {
    Msg {
        msg_type: c.t.clone(),
        msg_id: c.i.clone(),
        from: c.f.clone(),
        to: c.o.clone(),
        ts: c.s,
        nonce: c.n.clone(),
        hmac: c.h.clone(),
        payload: c.p.clone(),
    }
}
