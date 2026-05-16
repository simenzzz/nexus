use base64::Engine;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Transact, Update};

use crate::error::AppError;

/// Default ceiling on a single encoded Yjs update or full state we'll accept
/// from clients. Per-resource stores can raise it; the manager passes a
/// `max_bytes` into [`CollabDoc::apply_update_with_cap`] when applying.
pub const MAX_DOC_BYTES: usize = 256 * 1024;

/// Canonical text root name used by post documents. Whiteboards do not use
/// this — they store shapes in Y.Array/Y.Map roots defined client-side.
pub const TEXT_ROOT: &str = "content";

/// Thin wrapper over [`yrs::Doc`] that handles the base64-encoded Yjs v1 bytes
/// the WS protocol speaks. Resource-specific extraction (e.g., text for a
/// post, shape arrays for a whiteboard) is the caller's responsibility — the
/// doc itself stays generic over content shape.
pub struct CollabDoc {
    doc: Doc,
}

impl CollabDoc {
    fn from_doc(doc: Doc) -> Self {
        Self { doc }
    }

    /// Build an empty document.
    pub fn new() -> Self {
        Self::from_doc(Doc::new())
    }

    /// Hydrate from a previously persisted snapshot using the default cap.
    /// Empty input yields an empty doc.
    pub fn from_snapshot(state_b64: &str) -> Result<Self, AppError> {
        Self::from_snapshot_with_cap(state_b64, MAX_DOC_BYTES)
    }

    /// Hydrate with a caller-supplied byte cap. Resources whose live doc cap
    /// exceeds [`MAX_DOC_BYTES`] (whiteboards: 4 MB) MUST use this — using
    /// the default `from_snapshot` would refuse to load any already-persisted
    /// doc larger than 256 KB, permanently bricking it.
    pub fn from_snapshot_with_cap(state_b64: &str, max_bytes: usize) -> Result<Self, AppError> {
        let doc = Doc::new();
        if state_b64.is_empty() {
            return Ok(Self::from_doc(doc));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(state_b64)
            .map_err(|e| AppError::BadRequest(format!("Invalid snapshot base64: {e}")))?;
        if bytes.len() > max_bytes {
            return Err(AppError::BadRequest("Snapshot exceeds size limit".into()));
        }
        let update = Update::decode_v1(&bytes)
            .map_err(|e| AppError::BadRequest(format!("Invalid Yjs update: {e}")))?;
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(update)
                .map_err(|e| AppError::BadRequest(format!("Failed to apply update: {e}")))?;
        }
        Ok(Self::from_doc(doc))
    }

    /// Apply a remote update (base64 of Yjs v1 update bytes). Uses the
    /// default [`MAX_DOC_BYTES`] cap.
    pub fn apply_update(&self, update_b64: &str) -> Result<(), AppError> {
        self.apply_update_with_cap(update_b64, MAX_DOC_BYTES)
    }

    /// Apply a remote update with a caller-supplied byte cap. Whiteboards
    /// raise the cap to accommodate longer stroke updates.
    pub fn apply_update_with_cap(
        &self,
        update_b64: &str,
        max_bytes: usize,
    ) -> Result<(), AppError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(update_b64)
            .map_err(|e| AppError::BadRequest(format!("Invalid update base64: {e}")))?;
        if bytes.len() > max_bytes {
            return Err(AppError::BadRequest("Update exceeds size limit".into()));
        }
        let update = Update::decode_v1(&bytes)
            .map_err(|e| AppError::BadRequest(format!("Invalid Yjs update: {e}")))?;
        let mut txn = self.doc.transact_mut();
        txn.apply_update(update)
            .map_err(|e| AppError::BadRequest(format!("Failed to apply update: {e}")))
    }

    /// Encode the full document state as a Yjs v1 update (base64).
    pub fn encode_state(&self) -> String {
        let txn = self.doc.transact();
        let bytes = txn.encode_state_as_update_v1(&StateVector::default());
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Raw encoded state bytes — used by callers that need to enforce a
    /// post-merge document size cap (e.g. whiteboard 4 MB ceiling).
    pub fn encoded_state_len(&self) -> usize {
        let txn = self.doc.transact();
        txn.encode_state_as_update_v1(&StateVector::default()).len()
    }

    /// Encode the current state vector (base64). Clients use this to request
    /// a diff via `Y.encodeStateAsUpdate(doc, stateVector)`.
    pub fn encode_state_vector(&self) -> String {
        let txn = self.doc.transact();
        let bytes = txn.state_vector().encode_v1();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Extract the plain text from the canonical `content` text root.
    /// Used by [`PostStore::on_close`](super::post_store::PostStore) at
    /// publish time to freeze immutable `published_content`.
    pub fn text(&self) -> String {
        // `get_or_insert_text` is idempotent — safe to call even if the
        // client never bound to the root (returns "").
        let text = self.doc.get_or_insert_text(TEXT_ROOT);
        let txn = self.doc.transact();
        text.get_string(&txn)
    }

    #[cfg(test)]
    pub(crate) fn doc_ref(&self) -> &Doc {
        &self.doc
    }
}

impl Default for CollabDoc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use yrs::Text;

    /// Make a doc, insert some text, return its encoded full state.
    fn encode_with_text(text: &str) -> String {
        let doc = CollabDoc::new();
        let inner = doc.doc_ref().get_or_insert_text(TEXT_ROOT);
        let mut txn = doc.doc_ref().transact_mut();
        inner.insert(&mut txn, 0, text);
        drop(txn);
        doc.encode_state()
    }

    #[test]
    fn empty_doc_round_trips() {
        let doc = CollabDoc::new();
        assert_eq!(doc.text(), "");
        let snap = doc.encode_state();
        let restored = CollabDoc::from_snapshot(&snap).unwrap();
        assert_eq!(restored.text(), "");
    }

    #[test]
    fn snapshot_round_trip_preserves_text() {
        let snap = encode_with_text("hello world");
        let restored = CollabDoc::from_snapshot(&snap).unwrap();
        assert_eq!(restored.text(), "hello world");
    }

    #[test]
    fn merging_two_concurrent_updates_converges() {
        let doc_a = CollabDoc::new();
        {
            let t = doc_a.doc_ref().get_or_insert_text(TEXT_ROOT);
            let mut txn = doc_a.doc_ref().transact_mut();
            t.insert(&mut txn, 0, "abc");
        }
        let baseline = doc_a.encode_state();

        let doc_b = CollabDoc::from_snapshot(&baseline).unwrap();

        {
            let t = doc_a.doc_ref().get_or_insert_text(TEXT_ROOT);
            let mut txn = doc_a.doc_ref().transact_mut();
            t.insert(&mut txn, 3, "X");
        }
        {
            let t = doc_b.doc_ref().get_or_insert_text(TEXT_ROOT);
            let mut txn = doc_b.doc_ref().transact_mut();
            t.insert(&mut txn, 3, "Y");
        }

        let a_state = doc_a.encode_state();
        let b_state = doc_b.encode_state();
        doc_a.apply_update(&b_state).unwrap();
        doc_b.apply_update(&a_state).unwrap();

        assert_eq!(doc_a.text(), doc_b.text());
        let merged = doc_a.text();
        assert!(merged.contains('X'), "merged text {merged:?} missing X");
        assert!(merged.contains('Y'), "merged text {merged:?} missing Y");
    }

    #[test]
    fn apply_update_rejects_oversize_payload() {
        let doc = CollabDoc::new();
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_DOC_BYTES + 1]);
        let err = doc.apply_update(&big).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn apply_update_rejects_malformed_base64() {
        let doc = CollabDoc::new();
        let err = doc.apply_update("!!!not-base64!!!").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn snapshot_rejects_oversize_payload() {
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_DOC_BYTES + 1]);
        match CollabDoc::from_snapshot(&big) {
            Err(AppError::BadRequest(_)) => {}
            Err(other) => panic!("expected BadRequest, got {other:?}"),
            Ok(_) => panic!("expected BadRequest, got Ok"),
        }
    }

    #[test]
    fn apply_update_respects_caller_cap() {
        let doc = CollabDoc::new();
        // 300 bytes encoded — under default cap but over a 100-byte cap.
        let payload = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 300]);
        let err = doc.apply_update_with_cap(&payload, 100).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
