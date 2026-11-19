use blitz_dom::node::SpecialElementData;
use blitz_dom::{DocumentConfig, QualName, local_name, ns};
use blitz_html::HtmlDocument;
use blitz_traits::events::DomEventData;
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

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0,
    0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5, 1, 1,
    39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn document(html: &str) -> (HtmlDocument, Arc<ManualNetProvider>) {
    let net = Arc::new(ManualNetProvider::default());
    let doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url: Some("http://example.com/".to_string()),
            net_provider: Some(net.clone()),
            ..Default::default()
        },
    );
    (doc, net)
}

fn take_request(net: &ManualNetProvider, suffix: &str) -> Box<dyn NetHandler> {
    let mut requests = net.requests.lock().unwrap();
    let index = requests
        .iter()
        .position(|(url, _)| url.ends_with(suffix))
        .expect("image request");
    requests.remove(index).1
}

fn events(doc: &mut HtmlDocument) -> Vec<&'static str> {
    let mut events = Vec::new();
    while let Some(event) = doc.pop_pending_event() {
        events.push(match event.data {
            DomEventData::Load => "load",
            DomEventData::Error => "error",
            _ => continue,
        });
    }
    events
}

#[test]
fn valid_image_queues_load() {
    let (mut doc, net) = document(r#"<img src="a.png">"#);
    let handler = take_request(&net, "a.png");
    handler.bytes(
        "http://example.com/a.png".into(),
        Bytes::copy_from_slice(PNG),
    );
    doc.resolve(0.0);
    assert_eq!(events(&mut doc), ["load"]);
}

#[test]
fn failed_image_queues_error() {
    let (mut doc, net) = document(r#"<img src="missing.png">"#);
    let handler = take_request(&net, "missing.png");
    handler.error("http://example.com/missing.png".into(), "not found".into());
    doc.resolve(0.0);
    assert_eq!(events(&mut doc), ["error"]);
}

#[test]
fn cached_image_queues_load_without_fetch() {
    let (mut doc, net) = document(r#"<img id="first" src="a.png"><img id="second">"#);
    take_request(&net, "a.png").bytes(
        "http://example.com/a.png".into(),
        Bytes::copy_from_slice(PNG),
    );
    doc.resolve(0.0);
    events(&mut doc);

    let second = doc.get_element_by_id("second").unwrap();
    doc.mutate().set_attribute(
        second,
        QualName::new(None, ns!(), local_name!("src")),
        "a.png",
    );

    assert!(net.requests.lock().unwrap().is_empty());
    assert_eq!(events(&mut doc), ["load"]);
}

#[test]
fn stale_image_completion_does_not_apply_or_queue_load() {
    let (mut doc, net) = document(r#"<img id="image" src="a.png">"#);
    let image = doc.get_element_by_id("image").unwrap();
    doc.mutate().set_attribute(
        image,
        QualName::new(None, ns!(), local_name!("src")),
        "b.png",
    );

    take_request(&net, "a.png").bytes(
        "http://example.com/a.png".into(),
        Bytes::copy_from_slice(PNG),
    );
    doc.resolve(0.0);
    assert_eq!(events(&mut doc), Vec::<&str>::new());
    assert!(!matches!(
        doc.get_node(image)
            .unwrap()
            .element_data()
            .unwrap()
            .special_data,
        SpecialElementData::Image(_)
    ));

    take_request(&net, "b.png").bytes(
        "http://example.com/b.png".into(),
        Bytes::copy_from_slice(PNG),
    );
    doc.resolve(0.0);
    assert_eq!(events(&mut doc), ["load"]);
    assert_eq!(
        doc.get_node(image)
            .unwrap()
            .element_data()
            .unwrap()
            .attr(local_name!("src")),
        Some("b.png")
    );
}
