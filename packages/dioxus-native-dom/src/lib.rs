#![cfg_attr(docsrs, feature(doc_cfg))]

//! Core headless native renderer for Dioxus.
//!
//! ## Feature flags
//!  - `default`: Enables the features listed below.
//!  - `accessibility`: Enables [`accesskit`](https://docs.rs/accesskit/latest/accesskit/) accessibility support.
//!  - `hot-reload`: Enables hot-reloading of Dioxus RSX.
//!  - `menu`: Enables the [`muda`](https://docs.rs/muda/latest/muda/) menubar.
//!  - `tracing`: Enables tracing support.

mod dioxus_document;
mod events;
mod mutation_writer;
mod write_once_attr;
pub use blitz_dom::DocumentConfig;
pub use dioxus_document::DioxusDocument;
pub use events::{
    MouseEventButton, NativeFocusData, NativeFormData, NodeHandle, dispatch_synthetic_click,
    dispatch_synthetic_focus, dispatch_synthetic_input, dispatch_synthetic_key,
    dispatch_synthetic_pointer_down, dispatch_synthetic_pointer_move,
    dispatch_synthetic_pointer_up, dispatch_synthetic_wheel, parse_key_str, synthetic_click_event,
};
pub use write_once_attr::{CustomWidgetAttr, SubDocumentAttr};

pub use blitz_dom::NodeId;
use blitz_dom::{LocalName, Namespace, QualName, ns};

pub(crate) fn qual_name(local_name: &str, namespace: Option<&str>) -> QualName {
    QualName {
        prefix: None,
        ns: namespace.map(Namespace::from).unwrap_or(ns!(html)),
        local: LocalName::from(local_name),
    }
}

pub(crate) fn qual_name_attr(local_name: &str, namespace: Option<&str>) -> QualName {
    QualName {
        prefix: None,
        ns: namespace.map(Namespace::from).unwrap_or(ns!()),
        local: LocalName::from(local_name),
    }
}

// Syntax sugar to make tracing calls less noisy in function below
macro_rules! trace {
    ($pattern:literal) => {{
        #[cfg(feature = "tracing")]
        tracing::debug!($pattern);
    }};
    ($pattern:literal, $item1:expr) => {{
        #[cfg(feature = "tracing")]
        tracing::debug!($pattern, $item1);
    }};
    ($pattern:literal, $item1:expr, $item2:expr) => {{
        #[cfg(feature = "tracing")]
        tracing::debug!($pattern, $item1, $item2);
    }};
    ($pattern:literal, $item1:expr, $item2:expr, $item3:expr) => {{
        #[cfg(feature = "tracing")]
        tracing::debug!($pattern, $item1, $item2);
    }};
    ($pattern:literal, $item1:expr, $item2:expr, $item3:expr, $item4:expr) => {{
        #[cfg(feature = "tracing")]
        tracing::debug!($pattern, $item1, $item2, $item3, $item4);
    }};
}
pub(crate) use trace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_namespace_defaults_to_empty() {
        let attr_name = qual_name_attr("data-state", None);
        assert_eq!(attr_name.ns, ns!());
        assert_eq!(&*attr_name.local, "data-state");

        let elem_name = qual_name("div", None);
        assert_eq!(elem_name.ns, ns!(html));
        assert_eq!(&*elem_name.local, "div");
    }

    #[test]
    fn attribute_namespace_preserves_explicit_namespace() {
        let attr_name = qual_name_attr("href", Some("http://www.w3.org/1999/xlink"));
        assert_eq!(
            attr_name.ns,
            Namespace::from("http://www.w3.org/1999/xlink")
        );
        assert_eq!(&*attr_name.local, "href");
    }
}
