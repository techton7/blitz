//! Replaced elements with percentage `max-width` (e.g. Tailwind's `img { max-width: 100% }`)
//! must not collapse `min-width: auto` to 0 when a definite cross size transfers through
//! `aspect-ratio`, while preserving CSS Sizing 3 §5.2.2 `MinContent` compression (WPT
//! `css/css-flexbox/intrinsic-size/auto-min-size-001.html`) and inline-block shrink-to-fit
//! (`DioxusLabs/blitz#1432`).

use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use std::sync::Arc;

// Minimal valid 8x8 solid-red PNG.
const PNG_8X8: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x08, 0x02, 0x00, 0x00, 0x00, 0x4b, 0x6d, 0x29,
    0xdc, 0x00, 0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x80,
    0x15, 0x61, 0x17, 0x1d, 0xb4, 0x12, 0x00, 0x28, 0xff, 0x3f, 0xc1, 0x6e, 0xec, 0xdf, 0x61, 0x00,
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

// Minimal valid 64x64 solid-red PNG.
const PNG_64X64: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40, 0x08, 0x02, 0x00, 0x00, 0x00, 0x25, 0x0b, 0xe6,
    0x89, 0x00, 0x00, 0x00, 0x6f, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0xed, 0xcf, 0x01, 0x09, 0x00,
    0x00, 0x0c, 0x84, 0xc0, 0xef, 0x5f, 0x7a, 0x8b, 0x21, 0x82, 0x70, 0x01, 0x74, 0xb7, 0xa9, 0xf1,
    0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1,
    0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1,
    0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1,
    0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1,
    0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1, 0x05, 0x0d, 0xc8, 0xf1,
    0x05, 0x0d, 0xc8, 0x3d, 0xa9, 0xdc, 0xf0, 0xe2, 0xc3, 0x42, 0x2b, 0x71, 0x00, 0x00, 0x00, 0x00,
    0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

struct SyncPngNetProvider;

impl NetProvider for SyncPngNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        let bytes = if url.ends_with("8x8.png") {
            PNG_8X8
        } else {
            PNG_64X64
        };
        handler.bytes(url, Bytes::copy_from_slice(bytes));
    }
}

fn layout_doc(html: &str) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url: Some("http://example.com/".to_string()),
            net_provider: Some(Arc::new(SyncPngNetProvider)),
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    doc
}

fn size_of(doc: &HtmlDocument, selector: &str) -> (f32, f32) {
    let id = doc.query_selector(selector).unwrap().unwrap();
    let layout = doc.get_node(id).unwrap().final_layout();
    (layout.size.width, layout.size.height)
}

#[test]
fn inline_block_shrink_to_fit_percentage_image_preserved_1432() {
    let doc = layout_doc(
        r##"<body style="margin:0">
            <div id="col1" style="width:4px">
                <a id="ib1" href="#" style="display:inline-block">
                    <img id="img1" src="8x8.png" style="display:block;width:100%">
                </a>
            </div>
            <div id="col2" style="width:4px">
                <a id="ib2" href="#" style="display:inline-block">
                    <img id="img2" src="8x8.png" style="display:block;width:100%;max-width:100%">
                </a>
            </div>
        </body>"##,
    );
    assert_eq!(size_of(&doc, "#ib1"), (4.0, 4.0));
    assert_eq!(size_of(&doc, "#img1"), (4.0, 4.0));
    assert_eq!(size_of(&doc, "#ib2"), (4.0, 4.0));
    assert_eq!(size_of(&doc, "#img2"), (4.0, 4.0));
}

#[test]
fn flex_row_img_with_percentage_max_width_preserves_auto_min_width() {
    let doc = layout_doc(
        r#"<body style="margin:0">
            <div style="display:flex;width:100px;height:64px">
                <img id="img-definite-h" src="64x64.png" style="width:100%;height:100%;max-width:100%">
                <div style="width:80px;flex-shrink:0"></div>
            </div>
            <div style="display:flex;width:100px">
                <img id="img-auto-h" src="64x64.png" style="width:100%;height:100%;max-width:100%">
                <div style="width:80px;flex-shrink:0"></div>
            </div>
            <div style="display:flex;width:40px">
                <img id="img-narrow" src="64x64.png" style="width:100%;max-width:100%">
            </div>
        </body>"#,
    );
    assert_eq!(
        size_of(&doc, "#img-definite-h").0,
        64.0,
        "flex item img with definite cross size must preserve min-width: auto (64px)"
    );
    assert_eq!(
        size_of(&doc, "#img-auto-h").0,
        20.0,
        "flex item img with indefinite cross size and max-width: 100% compresses under MinContent (CSS Sizing 3 §5.2.2)"
    );
    assert_eq!(
        size_of(&doc, "#img-narrow").0,
        40.0,
        "flex item img in a narrow container must still clamp to container width (40px)"
    );
}
