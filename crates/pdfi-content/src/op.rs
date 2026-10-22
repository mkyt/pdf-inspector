//! Operators and the instruction list of a content stream.

use std::borrow::Cow;

use pdfi_syntax::{Dict, Node, Span};

/// Groups of operators, as in ISO 32000-2 Table 50.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    GeneralState,
    SpecialState,
    PathConstruction,
    PathPainting,
    Clipping,
    TextObject,
    TextState,
    TextPositioning,
    TextShowing,
    Type3,
    Color,
    Shading,
    InlineImage,
    XObject,
    MarkedContent,
    Compatibility,
    Unknown,
}

impl Category {
    pub const ALL: [Category; 17] = [
        Category::GeneralState,
        Category::SpecialState,
        Category::PathConstruction,
        Category::PathPainting,
        Category::Clipping,
        Category::TextObject,
        Category::TextState,
        Category::TextPositioning,
        Category::TextShowing,
        Category::Type3,
        Category::Color,
        Category::Shading,
        Category::InlineImage,
        Category::XObject,
        Category::MarkedContent,
        Category::Compatibility,
        Category::Unknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::GeneralState => "generalState",
            Category::SpecialState => "specialState",
            Category::PathConstruction => "pathConstruction",
            Category::PathPainting => "pathPainting",
            Category::Clipping => "clipping",
            Category::TextObject => "textObject",
            Category::TextState => "textState",
            Category::TextPositioning => "textPositioning",
            Category::TextShowing => "textShowing",
            Category::Type3 => "type3",
            Category::Color => "color",
            Category::Shading => "shading",
            Category::InlineImage => "inlineImage",
            Category::XObject => "xObject",
            Category::MarkedContent => "markedContent",
            Category::Compatibility => "compatibility",
            Category::Unknown => "unknown",
        }
    }
}

/// Kind of an operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    /// Integer or real.
    Num,
    Int,
    Name,
    Str,
    Array,
    /// A property list: a dictionary, or the name of one in `/Properties`.
    Props,
}

impl Arg {
    pub fn describe(self) -> &'static str {
        match self {
            Arg::Num => "number",
            Arg::Int => "integer",
            Arg::Name => "name",
            Arg::Str => "string",
            Arg::Array => "array",
            Arg::Props => "dictionary or name",
        }
    }
}

macro_rules! operators {
    ($($v:ident = $name:literal, $cat:ident, [$($arg:ident),*], $doc:literal;)*) => {
        /// A content stream operator. Unknown ones keep their spelling.
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum Operator {
            $(#[doc = $doc] $v,)*
            Unknown(Box<[u8]>),
        }

        impl Operator {
            pub fn from_bytes(b: &[u8]) -> Self {
                match b {
                    $($name => Operator::$v,)*
                    _ => Operator::Unknown(b.into()),
                }
            }

            /// Spelling in content streams.
            pub fn name(&self) -> Cow<'_, str> {
                match self {
                    $(Operator::$v => Cow::Borrowed(
                        // Operator names are ASCII.
                        match std::str::from_utf8($name) { Ok(s) => s, Err(_) => "" }
                    ),)*
                    Operator::Unknown(b) => String::from_utf8_lossy(b),
                }
            }

            pub fn category(&self) -> Category {
                match self {
                    $(Operator::$v => Category::$cat,)*
                    Operator::Unknown(_) => Category::Unknown,
                }
            }

            /// Operands it takes, in order. Empty for the color operators
            /// taking a variable number (see [`Operator::is_variadic`]).
            pub fn args(&self) -> &'static [Arg] {
                match self {
                    $(Operator::$v => &[$(Arg::$arg),*],)*
                    Operator::Unknown(_) => &[],
                }
            }

            /// What it does, in a few words.
            pub fn description(&self) -> &'static str {
                match self {
                    $(Operator::$v => $doc,)*
                    Operator::Unknown(_) => "unknown operator",
                }
            }
        }
    };
}

operators! {
    LineWidth = b"w", GeneralState, [Num], "set line width";
    LineCap = b"J", GeneralState, [Int], "set line cap style";
    LineJoin = b"j", GeneralState, [Int], "set line join style";
    MiterLimit = b"M", GeneralState, [Num], "set miter limit";
    Dash = b"d", GeneralState, [Array, Num], "set line dash pattern";
    Intent = b"ri", GeneralState, [Name], "set rendering intent";
    Flatness = b"i", GeneralState, [Num], "set flatness tolerance";
    ExtGState = b"gs", GeneralState, [Name], "set parameters from a graphics state parameter dictionary";
    Save = b"q", SpecialState, [], "save graphics state";
    Restore = b"Q", SpecialState, [], "restore graphics state";
    Concat = b"cm", SpecialState, [Num, Num, Num, Num, Num, Num], "concatenate matrix to the current transformation matrix";
    MoveTo = b"m", PathConstruction, [Num, Num], "begin new subpath";
    LineTo = b"l", PathConstruction, [Num, Num], "append straight line segment";
    CurveTo = b"c", PathConstruction, [Num, Num, Num, Num, Num, Num], "append Bézier curve (two control points)";
    CurveV = b"v", PathConstruction, [Num, Num, Num, Num], "append Bézier curve (first control point = current point)";
    CurveY = b"y", PathConstruction, [Num, Num, Num, Num], "append Bézier curve (second control point = end point)";
    ClosePath = b"h", PathConstruction, [], "close subpath";
    Rect = b"re", PathConstruction, [Num, Num, Num, Num], "append rectangle";
    Stroke = b"S", PathPainting, [], "stroke path";
    CloseStroke = b"s", PathPainting, [], "close and stroke path";
    Fill = b"f", PathPainting, [], "fill path (nonzero winding)";
    FillObsolete = b"F", PathPainting, [], "fill path (nonzero winding; obsolete spelling)";
    FillEvenOdd = b"f*", PathPainting, [], "fill path (even-odd)";
    FillStroke = b"B", PathPainting, [], "fill and stroke path (nonzero winding)";
    FillStrokeEvenOdd = b"B*", PathPainting, [], "fill and stroke path (even-odd)";
    CloseFillStroke = b"b", PathPainting, [], "close, fill and stroke path (nonzero winding)";
    CloseFillStrokeEvenOdd = b"b*", PathPainting, [], "close, fill and stroke path (even-odd)";
    EndPath = b"n", PathPainting, [], "end path without filling or stroking";
    Clip = b"W", Clipping, [], "set clipping path (nonzero winding)";
    ClipEvenOdd = b"W*", Clipping, [], "set clipping path (even-odd)";
    BeginText = b"BT", TextObject, [], "begin text object";
    EndText = b"ET", TextObject, [], "end text object";
    CharSpacing = b"Tc", TextState, [Num], "set character spacing";
    WordSpacing = b"Tw", TextState, [Num], "set word spacing";
    HorizontalScale = b"Tz", TextState, [Num], "set horizontal scaling (percent)";
    Leading = b"TL", TextState, [Num], "set text leading";
    Font = b"Tf", TextState, [Name, Num], "set text font and size";
    RenderMode = b"Tr", TextState, [Int], "set text rendering mode";
    Rise = b"Ts", TextState, [Num], "set text rise";
    TextMove = b"Td", TextPositioning, [Num, Num], "move text position";
    TextMoveLeading = b"TD", TextPositioning, [Num, Num], "move text position and set leading";
    TextMatrix = b"Tm", TextPositioning, [Num, Num, Num, Num, Num, Num], "set text matrix and text line matrix";
    NextLine = b"T*", TextPositioning, [], "move to start of next text line";
    ShowText = b"Tj", TextShowing, [Str], "show text";
    NextLineShow = b"'", TextShowing, [Str], "move to next line and show text";
    NextLineSpacingShow = b"\"", TextShowing, [Num, Num, Str], "set word and character spacing, move to next line and show text";
    ShowTextArray = b"TJ", TextShowing, [Array], "show text, allowing individual glyph positioning";
    GlyphWidth = b"d0", Type3, [Num, Num], "set glyph width (Type 3 font, colored glyph)";
    GlyphWidthBox = b"d1", Type3, [Num, Num, Num, Num, Num, Num], "set glyph width and bounding box (Type 3 font, uncolored glyph)";
    StrokeSpace = b"CS", Color, [Name], "set color space for stroking";
    FillSpace = b"cs", Color, [Name], "set color space for nonstroking";
    StrokeColor = b"SC", Color, [], "set color for stroking";
    StrokeColorN = b"SCN", Color, [], "set color for stroking (also Pattern, Separation, DeviceN, ICCBased)";
    FillColor = b"sc", Color, [], "set color for nonstroking";
    FillColorN = b"scn", Color, [], "set color for nonstroking (also Pattern, Separation, DeviceN, ICCBased)";
    StrokeGray = b"G", Color, [Num], "set gray level for stroking";
    FillGray = b"g", Color, [Num], "set gray level for nonstroking";
    StrokeRgb = b"RG", Color, [Num, Num, Num], "set RGB color for stroking";
    FillRgb = b"rg", Color, [Num, Num, Num], "set RGB color for nonstroking";
    StrokeCmyk = b"K", Color, [Num, Num, Num, Num], "set CMYK color for stroking";
    FillCmyk = b"k", Color, [Num, Num, Num, Num], "set CMYK color for nonstroking";
    Shade = b"sh", Shading, [Name], "paint area defined by shading pattern";
    InlineImage = b"BI", InlineImage, [], "inline image (BI … ID … EI)";
    ImageData = b"ID", InlineImage, [], "begin inline image data";
    EndImage = b"EI", InlineImage, [], "end inline image";
    XObject = b"Do", XObject, [Name], "paint XObject";
    MarkPoint = b"MP", MarkedContent, [Name], "define marked-content point";
    MarkPointProps = b"DP", MarkedContent, [Name, Props], "define marked-content point with property list";
    BeginMarked = b"BMC", MarkedContent, [Name], "begin marked-content sequence";
    BeginMarkedProps = b"BDC", MarkedContent, [Name, Props], "begin marked-content sequence with property list";
    EndMarked = b"EMC", MarkedContent, [], "end marked-content sequence";
    BeginCompat = b"BX", Compatibility, [], "begin compatibility section";
    EndCompat = b"EX", Compatibility, [], "end compatibility section";
}

impl Operator {
    /// The color operators taking as many operands as the color space has
    /// components (and, for `SCN` / `scn`, a pattern name).
    pub fn is_variadic(&self) -> bool {
        matches!(
            self,
            Operator::StrokeColor
                | Operator::StrokeColorN
                | Operator::FillColor
                | Operator::FillColorN
        )
    }
}

/// Where an operator was read from: a stream of the content and the bytes
/// of its operands and the operator in the decoded data of that stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpOrigin {
    /// Index in [`ContentProgram::sources`].
    pub source: u32,
    pub span: Span,
}

/// An inline image (`BI` dictionary `ID` data `EI`).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineImage {
    /// Entries as written (abbreviations not expanded).
    pub dict: Dict,
    /// The image data in the decoded stream data.
    pub data: Span,
}

/// One instruction: an operator and its operands.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub operator: Operator,
    /// Spans refer to the decoded data of the source stream.
    pub operands: Vec<Node>,
    /// `None` for operations made by edits.
    pub origin: Option<OpOrigin>,
    /// For `BI`.
    pub inline_image: Option<Box<InlineImage>>,
}

/// One stream of a content.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentSource {
    pub id: Option<pdfi_syntax::ObjId>,
    /// Length of the decoded data that was parsed.
    pub len: u64,
    pub error: Option<String>,
}

/// One logical content: a page's `/Contents` streams concatenated, or a
/// form XObject, pattern, glyph procedure or appearance stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentProgram {
    pub sources: Vec<ContentSource>,
    pub ops: Vec<Op>,
    /// Stopped at the operation limit.
    pub truncated: bool,
}
