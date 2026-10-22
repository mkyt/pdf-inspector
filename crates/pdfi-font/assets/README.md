# Substitute fonts

The `Foxit*.pfb` files are the standard fonts that PDFium ships
(`third_party/foxitfonts`), taken from the assets of hayro-interpret 0.8.0.
Despite the extension they are bare CFF fonts. License: BSD-3-Clause, see
`LICENSE_FOXIT` (copyright PDFium Authors; original code copyright Foxit
Software Inc.).

`HaranoAjiMincho-Regular.otf` and `HaranoAjiGothic-Medium.otf` (version
20250811, unmodified, from TeX Live's `haranoaji` package) draw CID fonts
that are not embedded. Harano Aji is made from Source Han Serif and Source
Han Sans (Adobe) with the glyphs in Adobe-Japan1-7 order. License: SIL
Open Font License 1.1, see `LICENSE_HARANOAJI` (copyright Adobe, with
Reserved Font Name 'Source', and the Harano Aji authors).
