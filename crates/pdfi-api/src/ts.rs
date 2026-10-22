//! TypeScript declarations of the protocol, generated from the Rust types.

use ts_rs::{Config, TS};

use crate::dto::*;
use crate::protocol::*;

/// Contents of `apps/web/src/protocol/generated.ts`.
pub fn typescript() -> String {
    // Offsets and counts stay far below 2^53, so plain numbers are exact.
    let cfg = Config::new().with_large_int("number");
    let decls = [
        ObjRef::decl(&cfg),
        Span::decl(&cfg),
        Range::decl(&cfg),
        Page::<()>::decl(&cfg),
        OpenSource::decl(&cfg),
        DirEntry::decl(&cfg),
        DirListing::decl(&cfg),
        DocInfo::decl(&cfg),
        DocSummary::decl(&cfg),
        EncryptionInfo::decl(&cfg),
        XrefKind::decl(&cfg),
        RevisionInfo::decl(&cfg),
        XrefEntry::decl(&cfg),
        XrefRow::decl(&cfg),
        ObjectSummary::decl(&cfg),
        NodeRoot::decl(&cfg),
        NodePath::decl(&cfg),
        StringKind::decl(&cfg),
        Value::decl(&cfg),
        NodeView::decl(&cfg),
        Location::decl(&cfg),
        StreamInfo::decl(&cfg),
        ObjectView::decl(&cfg),
        Severity::decl(&cfg),
        DiagnosticInfo::decl(&cfg),
        TypeOrigin::decl(&cfg),
        ObjectSchema::decl(&cfg),
        Requirement::decl(&cfg),
        SlotDoc::decl(&cfg),
        KeyDoc::decl(&cfg),
        NodeSchema::decl(&cfg),
        BytesInfo::decl(&cfg),
        JobKind::decl(&cfg),
        JobState::decl(&cfg),
        JobInfo::decl(&cfg),
        JobResult::decl(&cfg),
        ClientMessage::decl(&cfg),
        Request::decl(&cfg),
        Response::decl(&cfg),
        ResponseBody::decl(&cfg),
        HelloInfo::decl(&cfg),
        ErrorCode::decl(&cfg),
        ApiError::decl(&cfg),
        ServerMessage::decl(&cfg),
        Event::decl(&cfg),
    ];
    let mut out = String::from(
        "// Generated from crates/pdfi-api by `UPDATE_TS=1 cargo test -p pdfi-api`. Do not edit.\n\n",
    );
    out.push_str(&format!(
        "export const PROTOCOL_VERSION = {PROTOCOL_VERSION};\n\nexport type DocId = number;\nexport type JobId = number;\nexport type RequestId = number;\n\n"
    ));
    for d in decls {
        out.push_str("export ");
        out.push_str(&d);
        out.push_str("\n\n");
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}
