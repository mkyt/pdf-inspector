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
        TypeKey::decl(&cfg),
        NodeSchema::decl(&cfg),
        Rect::decl(&cfg),
        AttrOrigin::decl(&cfg),
        PageSummary::decl(&cfg),
        PageBox::decl(&cfg),
        ContentInfo::decl(&cfg),
        ResourceItem::decl(&cfg),
        PageDetail::decl(&cfg),
        Destination::decl(&cfg),
        LinkTarget::decl(&cfg),
        OutlineItem::decl(&cfg),
        AnnotInfo::decl(&cfg),
        AppearanceInfo::decl(&cfg),
        AppearancePick::decl(&cfg),
        InfoEntry::decl(&cfg),
        Feature::decl(&cfg),
        DocumentView::decl(&cfg),
        XmpNodeKind::decl(&cfg),
        XmpNode::decl(&cfg),
        XmpProperty::decl(&cfg),
        XmpView::decl(&cfg),
        FontInfo::decl(&cfg),
        FontKind::decl(&cfg),
        GlyphSourceKind::decl(&cfg),
        FontProgramInfo::decl(&cfg),
        FontDetail::decl(&cfg),
        GlyphView::decl(&cfg),
        ImageInfo::decl(&cfg),
        Referrer::decl(&cfg),
        ObjectClass::decl(&cfg),
        ObjectFilter::decl(&cfg),
        RefPath::decl(&cfg),
        PathStep::decl(&cfg),
        ContentRef::decl(&cfg),
        ContentKind::decl(&cfg),
        ContentSourceInfo::decl(&cfg),
        CategoryCount::decl(&cfg),
        ContentSummary::decl(&cfg),
        ViewInfo::decl(&cfg),
        PixelRect::decl(&cfg),
        RenderLayer::decl(&cfg),
        RenderNote::decl(&cfg),
        BitmapInfo::decl(&cfg),
        ContentIssue::decl(&cfg),
        ElementView::decl(&cfg),
        OpView::decl(&cfg),
        ColorView::decl(&cfg),
        FontView::decl(&cfg),
        GraphicsState::decl(&cfg),
        OpState::decl(&cfg),
        BytesInfo::decl(&cfg),
        JobKind::decl(&cfg),
        JobState::decl(&cfg),
        JobInfo::decl(&cfg),
        JobResult::decl(&cfg),
        InventoryStats::decl(&cfg),
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
