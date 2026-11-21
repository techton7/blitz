use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

/// Regression test for https://github.com/DioxusLabs/blitz/issues/407
///
/// `resolve_stylist` used to panic with "invalid key" when a CSS-animated node
/// was removed between two `resolve` calls. Verify the second resolve is safe.
#[test]
fn resolve_does_not_panic_after_removing_animated_node() {
    let html = r#"
        <style>
          @keyframes pulse { 0% { opacity: 1; } 100% { opacity: 0.5; } }
          .pulse { animation: pulse 2s infinite; }
        </style>
        <div id="pulse-node" class="pulse">animated</div>
    "#;

    let mut doc = HtmlDocument::from_html(html, DocumentConfig::default());
    doc.resolve(0.0);

    let pulse_id = doc.get_element_by_id("pulse-node");
    if let Some(id) = pulse_id {
        doc.mutate().remove_and_drop_node(id);
    }

    // Must not panic: stale animation entry should be skipped safely.
    doc.resolve(0.1);
}

#[test]
fn anim_inspection_timeline_and_critical_resource_early_return() {
    use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct ManualNetProvider {
        requests: Mutex<Vec<(String, Box<dyn NetHandler>)>>,
    }

    impl NetProvider for ManualNetProvider {
        fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
            self.requests
                .lock()
                .unwrap()
                .push((request.url.to_string(), handler));
        }
    }

    let net = Arc::new(ManualNetProvider::default());
    let mut doc = HtmlDocument::from_html(
        r#"<!doctype html>
        <html>
          <head><link rel="stylesheet" href="case.css"></head>
          <body>
            <style>
              @keyframes pulse { 0% { opacity: 1; } 100% { opacity: 0.5; } }
              .pulse { animation: pulse 2s infinite; }
            </style>
            <div id="pulse-node" class="pulse">animated</div>
          </body>
        </html>"#,
        DocumentConfig {
            base_url: Some("http://example.com/".to_string()),
            net_provider: Some(net.clone()),
            ..Default::default()
        },
    );

    assert!(doc.has_pending_critical_resources());
    let initial_gen = doc.resolve_generation();
    let initial_time = doc.last_animation_time();

    // Critical resource early return must not overwrite last_animation_time or increment resolve_generation.
    doc.resolve(99.0);
    assert_eq!(doc.last_animation_time(), initial_time);
    assert_eq!(doc.resolve_generation(), initial_gen);

    // Unblock critical stylesheet, then verify resolve(12.5) and resolve(13.0).
    let (url, handler) = net.requests.lock().unwrap().pop().unwrap();
    handler.bytes(url, Bytes::copy_from_slice(b"body { margin: 0; }"));

    doc.resolve(12.5);
    assert!(!doc.has_pending_critical_resources());
    assert_eq!(doc.last_animation_time(), 12.5);
    assert_eq!(doc.resolve_generation(), initial_gen + 1);

    let pulse_id = doc.get_element_by_id("pulse-node").unwrap();
    assert!(doc.node_has_active_animations(pulse_id));
    assert_eq!(doc.node_or_ancestor_has_display_none(pulse_id), Some(false));

    doc.resolve(13.0);
    assert_eq!(doc.last_animation_time(), 13.0);
    assert_eq!(doc.resolve_generation(), initial_gen + 2);
}
