use blitz_dom::{
    BaseDocument, Document, Node, ScrollBehavior as BlitzScrollBehavior,
    ScrollLogicalPosition as BlitzScrollLogicalPosition,
};
use blitz_traits::SmolStr;
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzScrollEvent, BlitzWheelDelta,
    BlitzWheelEvent, KeyState, MouseEventButton, UiEvent,
};
use dioxus_html::{
    AnimationData, CancelData, ClipboardData, CompositionData, DragData, FocusData, FormData,
    FormValue, HasFileData, HasFocusData, HasFormData, HasKeyboardData, HasMouseData,
    HasPointerData, HasScrollData, HasTouchData, HasTouchPointData, HasWheelData,
    HtmlEventConverter, ImageData, KeyboardData, MediaData, MountedData, MountedError,
    MountedResult, MouseData, PlatformEventData, PointerData, RenderedElementBacking, ResizeData,
    ScrollBehavior, ScrollData, ScrollLogicalPosition, ScrollToOptions, SelectionData, ToggleData,
    TouchData, TouchPoint, TransitionData, VisibleData, WheelData,
    geometry::{
        ClientPoint, ElementPoint, PagePoint, PixelsRect, PixelsSize, PixelsVector2D, ScreenPoint,
        WheelDelta,
        euclid::{Point2D, Size2D, Vector3D},
    },
    input_data::{MouseButton, MouseButtonSet},
    point_interaction::{
        InteractionElementOffset, InteractionLocation, ModifiersInteraction, PointerInteraction,
    },
};
use keyboard_types::{Code, Key, Location, Modifiers};
use std::{
    any::Any,
    cell::{Ref, RefCell, RefMut},
    fmt::Display,
    future::Future,
    pin::Pin,
    rc::Rc,
};

use crate::NodeId;

pub struct NativeConverter {}

impl HtmlEventConverter for NativeConverter {
    fn convert_cancel_data(&self, _event: &PlatformEventData) -> CancelData {
        unimplemented!("todo: convert_cancel_data in dioxus-native. requires support in blitz")
    }

    fn convert_form_data(&self, event: &PlatformEventData) -> FormData {
        event.downcast::<NativeFormData>().unwrap().clone().into()
    }

    fn convert_mouse_data(&self, event: &PlatformEventData) -> MouseData {
        event
            .downcast::<NativePointerData>()
            .unwrap()
            .clone()
            .into()
    }

    fn convert_keyboard_data(&self, event: &PlatformEventData) -> KeyboardData {
        event
            .downcast::<BlitzKeyboardData>()
            .unwrap()
            .clone()
            .into()
    }

    fn convert_focus_data(&self, _event: &PlatformEventData) -> FocusData {
        NativeFocusData {}.into()
    }

    fn convert_animation_data(&self, _event: &PlatformEventData) -> AnimationData {
        unimplemented!("todo: convert_animation_data in dioxus-native. requires support in blitz")
    }

    fn convert_clipboard_data(&self, _event: &PlatformEventData) -> ClipboardData {
        unimplemented!("todo: convert_clipboard_data in dioxus-native. requires support in blitz")
    }

    fn convert_composition_data(&self, _event: &PlatformEventData) -> CompositionData {
        unimplemented!("todo: convert_composition_data in dioxus-native. requires support in blitz")
    }

    fn convert_drag_data(&self, _event: &PlatformEventData) -> DragData {
        unimplemented!("todo: convert_drag_data in dioxus-native. requires support in blitz")
    }

    fn convert_image_data(&self, _event: &PlatformEventData) -> ImageData {
        unimplemented!("todo: convert_image_data in dioxus-native. requires support in blitz")
    }

    fn convert_media_data(&self, _event: &PlatformEventData) -> MediaData {
        unimplemented!("todo: convert_media_data in dioxus-native. requires support in blitz")
    }

    fn convert_mounted_data(&self, event: &PlatformEventData) -> MountedData {
        event.downcast::<NodeHandle>().unwrap().clone().into()
    }

    fn convert_pointer_data(&self, event: &PlatformEventData) -> PointerData {
        event
            .downcast::<NativePointerData>()
            .unwrap()
            .clone()
            .into()
    }

    fn convert_scroll_data(&self, event: &PlatformEventData) -> ScrollData {
        event.downcast::<NativeScrollData>().unwrap().clone().into()
    }

    fn convert_selection_data(&self, _event: &PlatformEventData) -> SelectionData {
        unimplemented!("todo: convert_selection_data in dioxus-native. requires support in blitz")
    }

    fn convert_toggle_data(&self, _event: &PlatformEventData) -> ToggleData {
        unimplemented!("todo: convert_toggle_data in dioxus-native. requires support in blitz")
    }

    fn convert_touch_data(&self, event: &PlatformEventData) -> TouchData {
        event.downcast::<NativeTouchData>().unwrap().clone().into()
    }

    fn convert_transition_data(&self, _event: &PlatformEventData) -> TransitionData {
        unimplemented!("todo: convert_transition_data in dioxus-native. requires support in blitz")
    }

    fn convert_wheel_data(&self, event: &PlatformEventData) -> WheelData {
        event.downcast::<NativeWheelData>().unwrap().clone().into()
    }

    fn convert_resize_data(&self, _event: &PlatformEventData) -> ResizeData {
        unimplemented!("todo: convert_resize_data in dioxus-native. requires support in blitz")
    }

    fn convert_visible_data(&self, _event: &PlatformEventData) -> VisibleData {
        unimplemented!("todo: convert_visible_data in dioxus-native. requires support in blitz")
    }
}

#[derive(Clone)]
pub struct NodeHandle {
    pub(crate) doc: Rc<RefCell<BaseDocument>>,
    pub(crate) node_id: NodeId,
}

impl NodeHandle {
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn doc(&self) -> Ref<'_, BaseDocument> {
        self.doc.borrow()
    }

    /// Returns `None` if the document is currently mutably borrowed.
    /// Use this from background tasks to avoid panicking during event handling.
    pub fn try_doc(&self) -> Option<Ref<'_, BaseDocument>> {
        self.doc.try_borrow().ok()
    }

    pub fn doc_mut(&self) -> RefMut<'_, BaseDocument> {
        self.doc.borrow_mut()
    }

    pub fn node(&self) -> Ref<'_, Node> {
        Ref::map(self.doc.borrow(), |doc| {
            doc.get_node(self.node_id)
                .expect("Node does not exist in the Document")
        })
    }

    pub fn node_mut(&self) -> RefMut<'_, Node> {
        RefMut::map(self.doc.borrow_mut(), |doc| {
            doc.get_node_mut(self.node_id)
                .expect("Node does not exist in the Document")
        })
    }

    fn node_not_exist_err<T>(&self) -> Pin<Box<dyn Future<Output = MountedResult<T>>>> {
        let node_id = self.node_id;
        let err = MountedError::OperationFailed(Box::new(NodeNotExistErr(node_id)));
        Box::pin(async move { Err(err) })
    }

    /// Dispatch a synthetic click on this node using the native DOM event plumbing.
    pub fn click(&self) -> bool {
        dispatch_synthetic_click(&self.doc(), self.node_id, Modifiers::empty())
    }

    /// Dispatch a synthetic focus on this node using the native DOM event plumbing.
    pub fn focus_synthetic(&self) -> bool {
        dispatch_synthetic_focus(&mut self.doc_mut(), self.node_id)
    }

    /// Dispatch a synthetic input / value change on this node using the native DOM event plumbing.
    pub fn set_value_synthetic(&self, value: &str) -> bool {
        dispatch_synthetic_input(&mut self.doc_mut(), self.node_id, value)
    }
}

#[derive(Debug)]
struct NodeNotExistErr(NodeId);
impl Display for NodeNotExistErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The node {} does not exist", self.0)
    }
}
impl std::error::Error for NodeNotExistErr {}

impl RenderedElementBacking for NodeHandle {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn get_scroll_offset(&self) -> Pin<Box<dyn Future<Output = MountedResult<PixelsVector2D>>>> {
        let scroll_offset = *self.node().scroll_offset();
        Box::pin(async move { Ok(PixelsVector2D::new(scroll_offset.x, scroll_offset.y)) })
    }

    fn get_scroll_size(&self) -> Pin<Box<dyn Future<Output = MountedResult<PixelsSize>>>> {
        let node = self.node();
        let scroll_width = node.final_layout().scroll_width() as f64;
        let scroll_height = node.final_layout().scroll_height() as f64;
        Box::pin(async move { Ok(PixelsSize::new(scroll_width, scroll_height)) })
    }

    fn get_client_rect(&self) -> Pin<Box<dyn Future<Output = MountedResult<PixelsRect>>>> {
        let Some(bounding_rect) = self.doc_mut().get_client_bounding_rect(self.node_id) else {
            return self.node_not_exist_err();
        };
        let pixels_rect = PixelsRect::new(
            Point2D::new(bounding_rect.x, bounding_rect.y),
            Size2D::new(bounding_rect.width, bounding_rect.height),
        );
        Box::pin(async move { Ok(pixels_rect) })
    }

    /// Scroll the element into view (the backing for both `MountedData::scroll_to` and
    /// `MountedData::scroll_to_with_options`).
    fn scroll_to(
        &self,
        options: ScrollToOptions,
    ) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        let node_id = self.node_id;
        let mut doc = self.doc_mut();
        if doc.get_node(node_id).is_none() {
            drop(doc);
            return self.node_not_exist_err();
        }

        let behavior = match options.behavior {
            ScrollBehavior::Smooth => BlitzScrollBehavior::Smooth,
            ScrollBehavior::Instant => BlitzScrollBehavior::Instant,
        };
        let vertical = match options.vertical {
            ScrollLogicalPosition::Start => BlitzScrollLogicalPosition::Start,
            ScrollLogicalPosition::Center => BlitzScrollLogicalPosition::Center,
            ScrollLogicalPosition::End => BlitzScrollLogicalPosition::End,
            ScrollLogicalPosition::Nearest => BlitzScrollLogicalPosition::Nearest,
        };
        let horizontal = match options.horizontal {
            ScrollLogicalPosition::Start => BlitzScrollLogicalPosition::Start,
            ScrollLogicalPosition::Center => BlitzScrollLogicalPosition::Center,
            ScrollLogicalPosition::End => BlitzScrollLogicalPosition::End,
            ScrollLogicalPosition::Nearest => BlitzScrollLogicalPosition::Nearest,
        };
        doc.scroll_into_view(node_id, behavior, vertical, horizontal);

        Box::pin(async { Ok(()) })
    }

    /// Scroll the element's own content to the given offset (the backing for
    /// `MountedData::scroll`). The offset is clamped to the element's scrollable range.
    fn scroll(
        &self,
        coordinates: PixelsVector2D,
        behavior: ScrollBehavior,
    ) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        let node_id = self.node_id;
        let mut doc = self.doc_mut();
        if doc.get_node(node_id).is_none() {
            drop(doc);
            return self.node_not_exist_err();
        }

        doc.scroll_to(
            node_id,
            coordinates.x,
            coordinates.y,
            match behavior {
                ScrollBehavior::Smooth => BlitzScrollBehavior::Smooth,
                ScrollBehavior::Instant => BlitzScrollBehavior::Instant,
            },
        );

        Box::pin(async { Ok(()) })
    }

    fn set_focus(&self, focus: bool) -> Pin<Box<dyn Future<Output = MountedResult<()>>>> {
        let mut doc = self.doc_mut();
        if focus {
            // TODO: queue focus events somehow
            doc.set_focus_to(self.node_id);
        } else if doc.get_focussed_node_id() == Some(self.node_id) {
            // Q: Should this only clear focus if the node is focussed?
            // TODO: queue blur events somehow
            doc.clear_focus();
        }

        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone, Debug)]
pub struct NativeFormData {
    pub value: String,
    pub values: Vec<(String, FormValue)>,
}

impl HasFormData for NativeFormData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn value(&self) -> String {
        self.value.clone()
    }

    fn values(&self) -> Vec<(String, FormValue)> {
        self.values.clone()
    }
    fn valid(&self) -> bool {
        // todo: actually implement validation here.
        true
    }
}

impl HasFileData for NativeFormData {
    fn files(&self) -> Vec<dioxus_html::FileData> {
        vec![]
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BlitzKeyboardData(pub(crate) BlitzKeyEvent);

impl ModifiersInteraction for BlitzKeyboardData {
    fn modifiers(&self) -> Modifiers {
        self.0.modifiers
    }
}

impl HasKeyboardData for BlitzKeyboardData {
    fn key(&self) -> Key {
        self.0.key.clone()
    }

    fn code(&self) -> Code {
        self.0.code
    }

    fn location(&self) -> Location {
        self.0.location
    }

    fn is_auto_repeating(&self) -> bool {
        self.0.is_auto_repeating
    }

    fn is_composing(&self) -> bool {
        self.0.is_composing
    }

    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

#[derive(Clone)]
pub struct NativePointerData(pub(crate) BlitzPointerEvent);

impl InteractionLocation for NativePointerData {
    fn client_coordinates(&self) -> ClientPoint {
        ClientPoint::new(self.0.client_x() as f64, self.0.client_y() as f64)
    }

    fn screen_coordinates(&self) -> ScreenPoint {
        ScreenPoint::new(self.0.screen_x() as f64, self.0.screen_y() as f64)
    }

    fn page_coordinates(&self) -> PagePoint {
        PagePoint::new(self.0.page_x() as f64, self.0.page_y() as f64)
    }
}

impl InteractionElementOffset for NativePointerData {
    fn element_coordinates(&self) -> ElementPoint {
        ElementPoint::new(self.0.element_x() as f64, self.0.element_y() as f64)
    }
}

impl ModifiersInteraction for NativePointerData {
    fn modifiers(&self) -> Modifiers {
        self.0.mods
    }
}

impl PointerInteraction for NativePointerData {
    fn trigger_button(&self) -> Option<MouseButton> {
        Some(match self.0.button {
            MouseEventButton::Main => MouseButton::Primary,
            MouseEventButton::Auxiliary => MouseButton::Auxiliary,
            MouseEventButton::Secondary => MouseButton::Secondary,
            MouseEventButton::Fourth => MouseButton::Fourth,
            MouseEventButton::Fifth => MouseButton::Fifth,
        })
    }

    fn held_buttons(&self) -> MouseButtonSet {
        dioxus_html::input_data::decode_mouse_button_set(self.0.buttons.bits() as u16)
    }
}
impl HasMouseData for NativePointerData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

impl HasPointerData for NativePointerData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn is_primary(&self) -> bool {
        self.0.is_primary
    }

    fn pointer_id(&self) -> i32 {
        match self.0.id {
            BlitzPointerId::Mouse => 0,
            BlitzPointerId::Pen => 0,
            BlitzPointerId::Finger(id) => id as i32,
        }
    }

    fn pointer_type(&self) -> String {
        match self.0.id {
            BlitzPointerId::Mouse => String::from("mouse"),
            BlitzPointerId::Pen => String::from("pen"),
            BlitzPointerId::Finger(_) => String::from("touch"),
        }
    }

    fn pressure(&self) -> f32 {
        self.0.details.pressure as f32
    }
    fn tangential_pressure(&self) -> f32 {
        self.0.details.tangential_pressure
    }
    fn tilt_x(&self) -> i32 {
        self.0.details.tilt_x as i32
    }
    fn tilt_y(&self) -> i32 {
        self.0.details.tilt_y as i32
    }
    fn twist(&self) -> i32 {
        self.0.details.twist as i32
    }

    // TODO: implement these fields with real values
    fn width(&self) -> f64 {
        1.0
    }
    fn height(&self) -> f64 {
        1.0
    }
}

/// Touch event data exposed to Dioxus Native application code.
///
/// Blitz tracks input via pointer events, so a touch event is generated from the
/// pointer event of the finger that triggered it (`touches_changed`). The full
/// list of concurrent touches is carried on the triggering event's
/// [`BlitzPointerEvent::active_pointers`] list and reported via `touches`.
#[derive(Clone)]
pub struct NativeTouchData(pub(crate) BlitzPointerEvent);

impl ModifiersInteraction for NativeTouchData {
    fn modifiers(&self) -> Modifiers {
        self.0.mods
    }
}

impl HasTouchData for NativeTouchData {
    fn touches(&self) -> Vec<TouchPoint> {
        // All pointers currently active on the surface (multi-touch).
        self.0
            .active_pointers
            .borrow()
            .iter()
            .map(|event| TouchPoint::new(NativeTouchPointData(event.clone())))
            .collect()
    }

    fn touches_changed(&self) -> Vec<TouchPoint> {
        // Just the touch that triggered this event.
        vec![TouchPoint::new(NativeTouchPointData(self.0.clone()))]
    }

    fn target_touches(&self) -> Vec<TouchPoint> {
        // We don't track a per-touch target, so approximate `targetTouches`
        // (touches that started on the event target) with all active touches.
        self.touches()
    }

    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

#[derive(Clone)]
pub struct NativeTouchPointData(BlitzPointerEvent);

impl InteractionLocation for NativeTouchPointData {
    fn client_coordinates(&self) -> ClientPoint {
        ClientPoint::new(self.0.client_x() as f64, self.0.client_y() as f64)
    }

    fn screen_coordinates(&self) -> ScreenPoint {
        ScreenPoint::new(self.0.screen_x() as f64, self.0.screen_y() as f64)
    }

    fn page_coordinates(&self) -> PagePoint {
        PagePoint::new(self.0.page_x() as f64, self.0.page_y() as f64)
    }
}

impl HasTouchPointData for NativeTouchPointData {
    fn identifier(&self) -> i32 {
        match self.0.id {
            BlitzPointerId::Finger(id) => id as i32,
            BlitzPointerId::Mouse | BlitzPointerId::Pen => 0,
        }
    }

    fn force(&self) -> f64 {
        self.0.details.pressure
    }

    fn radius(&self) -> ScreenPoint {
        // TODO: expose real touch radius once blitz tracks it
        ScreenPoint::new(1.0, 1.0)
    }

    fn rotation(&self) -> f64 {
        // TODO: expose real touch rotation once blitz tracks it
        0.0
    }

    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

#[derive(Clone)]
pub struct NativeFocusData;
impl HasFocusData for NativeFocusData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

#[derive(Clone)]
pub struct NativeScrollData(pub(crate) BlitzScrollEvent);
impl HasScrollData for NativeScrollData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn scroll_top(&self) -> f64 {
        self.0.scroll_top
    }

    fn scroll_left(&self) -> f64 {
        self.0.scroll_left
    }

    fn scroll_width(&self) -> i32 {
        self.0.scroll_width
    }

    fn scroll_height(&self) -> i32 {
        self.0.scroll_height
    }

    fn client_width(&self) -> i32 {
        self.0.client_width
    }

    fn client_height(&self) -> i32 {
        self.0.client_height
    }
}

#[derive(Clone)]
pub struct NativeWheelData(pub(crate) BlitzWheelEvent);
impl HasWheelData for NativeWheelData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn delta(&self) -> WheelDelta {
        match self.0.delta {
            BlitzWheelDelta::Lines(x, y) => {
                dioxus_html::geometry::WheelDelta::Lines(Vector3D::new(x, y, 0.0))
            }
            BlitzWheelDelta::Pixels(x, y) => {
                dioxus_html::geometry::WheelDelta::Pixels(Vector3D::new(x, y, 0.0))
            }
        }
    }
}

impl HasMouseData for NativeWheelData {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

impl PointerInteraction for NativeWheelData {
    fn trigger_button(&self) -> Option<MouseButton> {
        None
    }

    fn held_buttons(&self) -> MouseButtonSet {
        dioxus_html::input_data::decode_mouse_button_set(self.0.buttons.bits() as u16)
    }
}

impl ModifiersInteraction for NativeWheelData {
    fn modifiers(&self) -> Modifiers {
        self.0.mods
    }
}

impl InteractionElementOffset for NativeWheelData {
    fn element_coordinates(&self) -> ElementPoint {
        ElementPoint::new(self.0.element_x() as f64, self.0.element_y() as f64)
    }
}

impl InteractionLocation for NativeWheelData {
    fn client_coordinates(&self) -> ClientPoint {
        ClientPoint::new(self.0.client_x() as f64, self.0.client_y() as f64)
    }

    fn screen_coordinates(&self) -> ScreenPoint {
        ScreenPoint::new(self.0.screen_x() as f64, self.0.screen_y() as f64)
    }

    fn page_coordinates(&self) -> PagePoint {
        PagePoint::new(self.0.page_x() as f64, self.0.page_y() as f64)
    }
}

pub fn synthetic_click_event(node: &Node, modifiers: Modifiers) -> Box<dyn Any> {
    Box::new(NativePointerData(
        node.synthetic_click_event_data(modifiers),
    ))
}

/// Dispatch a synthetic click event on a target node in `doc`, bubbling up the DOM hierarchy
/// to locate the nearest Dioxus element listener.
pub fn dispatch_synthetic_click(
    doc: &BaseDocument,
    node_id: NodeId,
    modifiers: Modifiers,
) -> bool {
    let mut current = Some(node_id);
    let mut target = None;
    while let Some(id) = current {
        if let Some(node) = doc.get_node(id) {
            if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                target = Some((dioxus_id, id));
                break;
            }
            current = node.parent;
        } else {
            break;
        }
    }

    let Some((dioxus_id, target_node_id)) = target else {
        return false;
    };

    let Some(target_node) = doc.get_node(target_node_id) else {
        return false;
    };

    let event_data = synthetic_click_event(target_node, modifiers);
    let platform_event = PlatformEventData::new(event_data);
    let dx_event = dioxus_core::Event::new(Rc::new(platform_event) as Rc<dyn Any>, true);

    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
        runtime.handle_event("click", dx_event, dioxus_id);
        true
    } else {
        false
    }
}

/// Dispatch a synthetic focus event on a target node in `doc`.
/// Sets focus on the Blitz DOM node and dispatches "focus" and "focusin" events to Dioxus listeners.
pub fn dispatch_synthetic_focus(
    doc: &mut BaseDocument,
    node_id: NodeId,
) -> bool {
    let mut current = Some(node_id);
    let mut target = None;
    while let Some(id) = current {
        if let Some(node) = doc.get_node(id) {
            if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                target = Some((dioxus_id, id));
                break;
            }
            current = node.parent;
        } else {
            break;
        }
    }

    let Some((dioxus_id, target_node_id)) = target else {
        return false;
    };

    // Update Blitz DOM focus state
    let _ = doc.set_focus_to(target_node_id);

    // Dispatch Dioxus focus events
    let event_data = Box::new(NativeFocusData);
    let platform_event = Rc::new(PlatformEventData::new(event_data));
    let dx_focus = dioxus_core::Event::new(platform_event.clone() as Rc<dyn Any>, false);

    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
        runtime.handle_event("focus", dx_focus, dioxus_id);
        let dx_focusin = dioxus_core::Event::new(platform_event as Rc<dyn Any>, true);
        runtime.handle_event("focusin", dx_focusin, dioxus_id);
        true
    } else {
        true
    }
}

/// Dispatch a synthetic input/value change on a target node in `doc`.
/// Updates the Blitz DOM text input editor (if applicable) and dispatches "input" and "change"
/// events containing the new value to Dioxus listeners.
pub fn dispatch_synthetic_input(
    doc: &mut BaseDocument,
    node_id: NodeId,
    value: &str,
) -> bool {
    let mut current = Some(node_id);
    let mut target = None;
    while let Some(id) = current {
        if let Some(node) = doc.get_node(id) {
            if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                target = Some((dioxus_id, id));
                break;
            }
            current = node.parent;
        } else {
            break;
        }
    }

    let Some((dioxus_id, target_node_id)) = target else {
        return false;
    };

    // If target node is a text input, update its editor buffer and refresh layout
    doc.set_text_input_value(target_node_id, value);

    // Dispatch Dioxus input and change events
    let form_data = Box::new(NativeFormData {
        value: value.to_string(),
        values: vec![],
    });
    let platform_event = Rc::new(PlatformEventData::new(form_data));
    let dx_input = dioxus_core::Event::new(platform_event.clone() as Rc<dyn Any>, true);

    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
        runtime.handle_event("input", dx_input, dioxus_id);
        let dx_change = dioxus_core::Event::new(platform_event as Rc<dyn Any>, true);
        runtime.handle_event("change", dx_change, dioxus_id);
        true
    } else {
        true
    }
}

/// Helper to map key string to (Key, Code, Modifiers)
pub fn parse_key_str(raw: &str) -> (Key, Code, Modifiers) {
    let mut mods = Modifiers::empty();
    let mut key_part = raw.trim();

    while let Some(idx) = key_part.find('+') {
        let prefix = key_part[..idx].trim();
        if prefix.eq_ignore_ascii_case("shift") {
            mods |= Modifiers::SHIFT;
        } else if prefix.eq_ignore_ascii_case("ctrl") || prefix.eq_ignore_ascii_case("control") {
            mods |= Modifiers::CONTROL;
        } else if prefix.eq_ignore_ascii_case("cmd")
            || prefix.eq_ignore_ascii_case("meta")
            || prefix.eq_ignore_ascii_case("super")
        {
            mods |= Modifiers::SUPER;
        } else if prefix.eq_ignore_ascii_case("alt") || prefix.eq_ignore_ascii_case("opt") {
            mods |= Modifiers::ALT;
        }
        key_part = key_part[idx + 1..].trim();
    }

    let (key, code) = match key_part {
        "Tab" => (Key::Tab, Code::Tab),
        "Enter" => (Key::Enter, Code::Enter),
        "Space" | " " => (Key::Character(" ".into()), Code::Space),
        "Escape" | "Esc" => (Key::Escape, Code::Escape),
        "Backspace" => (Key::Backspace, Code::Backspace),
        "Delete" | "Del" => (Key::Delete, Code::Delete),
        "ArrowLeft" => (Key::ArrowLeft, Code::ArrowLeft),
        "ArrowRight" => (Key::ArrowRight, Code::ArrowRight),
        "ArrowUp" => (Key::ArrowUp, Code::ArrowUp),
        "ArrowDown" => (Key::ArrowDown, Code::ArrowDown),
        "a" | "A" => (Key::Character(key_part.to_string()), Code::KeyA),
        "c" | "C" => (Key::Character(key_part.to_string()), Code::KeyC),
        "v" | "V" => (Key::Character(key_part.to_string()), Code::KeyV),
        "x" | "X" => (Key::Character(key_part.to_string()), Code::KeyX),
        "z" | "Z" => (Key::Character(key_part.to_string()), Code::KeyZ),
        other => {
            if other.chars().count() == 1 {
                (Key::Character(other.to_string()), Code::Unidentified)
            } else {
                (Key::Unidentified, Code::Unidentified)
            }
        }
    };

    (key, code, mods)
}

/// Dispatch a synthetic keyboard action on a target node in `doc`.
/// Drives real Blitz DOM keyboard handling, Dioxus VirtualDom keyboard events,
/// and reactive updates for Focus/Activation, Text Editing, Navigation, and Modifiers.
pub fn dispatch_synthetic_key(
    doc: &mut BaseDocument,
    node_id: Option<NodeId>,
    key_str: &str,
    modifiers: Modifiers,
) -> Result<NodeId, String> {
    let (key, code, parsed_mods) = parse_key_str(key_str);
    let combined_mods = modifiers | parsed_mods;

    let target_node_id = match node_id {
        Some(nid) => {
            if doc.get_focussed_node_id() != Some(nid) {
                doc.set_focus_to(nid);
            }
            nid
        }
        None => doc
            .get_focussed_node_id()
            .unwrap_or_else(|| doc.root_node().id),
    };

    // 1. Handle Tab / Shift+Tab focus traversal
    if key == Key::Tab {
        let old_focus = doc.get_focussed_node_id();
        let new_focus = if combined_mods.contains(Modifiers::SHIFT) {
            doc.focus_prev_node()
        } else {
            doc.focus_next_node()
        };

        if let Some(new_id) = new_focus {
            if let Some(old_id) = old_focus {
                if old_id != new_id {
                    if let Some(node) = doc.get_node(old_id) {
                        if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                            let event_data = Box::new(NativeFocusData);
                            let platform_event = Rc::new(PlatformEventData::new(event_data));
                            let dx_blur = dioxus_core::Event::new(platform_event as Rc<dyn Any>, false);
                            if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
                                runtime.handle_event("blur", dx_blur, dioxus_id);
                            }
                        }
                    }
                }
            }

            if let Some(node) = doc.get_node(new_id) {
                if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                    let event_data = Box::new(NativeFocusData);
                    let platform_event = Rc::new(PlatformEventData::new(event_data));
                    let dx_focus = dioxus_core::Event::new(platform_event.clone() as Rc<dyn Any>, false);
                    let dx_focusin = dioxus_core::Event::new(platform_event as Rc<dyn Any>, true);
                    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
                        runtime.handle_event("focus", dx_focus, dioxus_id);
                        runtime.handle_event("focusin", dx_focusin, dioxus_id);
                    }
                }
            }
            return Ok(new_id);
        }
        return Ok(target_node_id);
    }

    // 2. Handle Escape focus clearing
    if key == Key::Escape {
        if let Some(old_id) = doc.get_focussed_node_id() {
            doc.clear_focus();
            if let Some(node) = doc.get_node(old_id) {
                if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                    let event_data = Box::new(NativeFocusData);
                    let platform_event = Rc::new(PlatformEventData::new(event_data));
                    let dx_blur = dioxus_core::Event::new(platform_event as Rc<dyn Any>, false);
                    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
                        runtime.handle_event("blur", dx_blur, dioxus_id);
                    }
                }
            }
        }
        return Ok(target_node_id);
    }

    // 3. Handle Enter / Space button activation
    let is_button = doc.get_node(target_node_id).and_then(|n| n.element_data()).is_some_and(|el| {
        el.name.local.as_ref() == "button"
            || el.attrs().iter().any(|a| a.name.local.as_ref() == "role" && a.value == "button")
    });
    if is_button && (key == Key::Enter || key == Key::Character(" ".into())) {
        dispatch_synthetic_click(doc, target_node_id, combined_mods);
    }

    // 4. Construct BlitzKeyEvent for KeyDown and KeyUp
    let text_payload = match &key {
        Key::Character(s) => Some(SmolStr::new(s)),
        _ => None,
    };

    let key_down = BlitzKeyEvent {
        key: key.clone(),
        code,
        modifiers: combined_mods,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: text_payload,
    };

    let key_up = BlitzKeyEvent {
        key: key.clone(),
        code,
        modifiers: combined_mods,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Released,
        text: None,
    };

    // 5. Dispatch UI events to BaseDocument
    doc.handle_ui_event(UiEvent::KeyDown(key_down.clone()));
    doc.handle_ui_event(UiEvent::KeyUp(key_up.clone()));

    // 6. If target node is a text input, propagate updated value to Dioxus listeners
    if let Some(node) = doc.get_node(target_node_id) {
        if let Some(el) = node.element_data() {
            if let Some(input_data) = el.text_input_data() {
                let current_val = input_data.editor.raw_text().to_string();
                let form_data = Box::new(NativeFormData {
                    value: current_val,
                    values: vec![],
                });
                let platform_event = Rc::new(PlatformEventData::new(form_data));
                let dx_input = dioxus_core::Event::new(platform_event.clone() as Rc<dyn Any>, true);
                let dx_change = dioxus_core::Event::new(platform_event as Rc<dyn Any>, true);

                if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                    if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
                        runtime.handle_event("input", dx_input, dioxus_id);
                        runtime.handle_event("change", dx_change, dioxus_id);
                    }
                }
            }
        }
    }

    // 7. Dispatch Dioxus KeyDown and KeyUp events
    let mut current = Some(target_node_id);
    while let Some(id) = current {
        if let Some(node) = doc.get_node(id) {
            if let Some(dioxus_id) = crate::dioxus_document::get_dioxus_id(node) {
                let kb_down = Box::new(BlitzKeyboardData(key_down.clone()));
                let platform_down = Rc::new(PlatformEventData::new(kb_down));
                let dx_down = dioxus_core::Event::new(platform_down as Rc<dyn Any>, true);

                let kb_up = Box::new(BlitzKeyboardData(key_up.clone()));
                let platform_up = Rc::new(PlatformEventData::new(kb_up));
                let dx_up = dioxus_core::Event::new(platform_up as Rc<dyn Any>, true);

                if let Ok(runtime) = std::panic::catch_unwind(dioxus_core::Runtime::current) {
                    runtime.handle_event("keydown", dx_down, dioxus_id);
                    runtime.handle_event("keyup", dx_up, dioxus_id);
                }
                break;
            }
            current = node.parent;
        } else {
            break;
        }
    }

    Ok(target_node_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_traits::events::{
        BlitzPointerId, MouseEventButton, MouseEventButtons, Point, PointerCoords, PointerDetails,
    };

    fn finger_event(id: u64, x: f32, y: f32) -> BlitzPointerEvent {
        BlitzPointerEvent {
            id: BlitzPointerId::Finger(id),
            is_primary: id == 0,
            coords: PointerCoords {
                page_x: x,
                page_y: y,
                screen_x: x,
                screen_y: y,
                client_x: x,
                client_y: y,
            },
            button: MouseEventButton::Main,
            buttons: MouseEventButtons::from(MouseEventButton::Main),
            mods: Default::default(),
            details: PointerDetails::default(),
            element: Point::default(),
            active_pointers: Default::default(),
        }
    }

    #[test]
    fn touches_reports_all_active_pointers() {
        let f0 = finger_event(0, 10.0, 20.0);
        let f1 = finger_event(1, 30.0, 40.0);

        // The triggering event (second finger down) carries the list of all
        // currently-active pointers.
        let trigger = f1.clone();
        {
            let mut list = trigger.active_pointers.borrow_mut();
            list.push(f0.clone());
            list.push(f1.clone());
        }

        let data = NativeTouchData(trigger);

        // `touches` reports every active pointer ...
        let touches = data.touches();
        assert_eq!(touches.len(), 2);
        let coords: Vec<(f64, f64)> = touches
            .iter()
            .map(|t| {
                let c = t.client_coordinates();
                (c.x, c.y)
            })
            .collect();
        assert!(coords.contains(&(10.0, 20.0)));
        assert!(coords.contains(&(30.0, 40.0)));

        // ... while `changed_touches` reports only the triggering touch.
        assert_eq!(data.touches_changed().len(), 1);
        let changed = data.touches_changed();
        let changed_coords = changed[0].client_coordinates();
        assert_eq!((changed_coords.x, changed_coords.y), (30.0, 40.0));
    }

    #[test]
    fn touches_is_empty_when_no_pointers_are_active() {
        // e.g. a `touchend` for the last finger: it has been removed from the
        // active list before dispatch, so `touches` is empty but
        // `changed_touches` still reports the finger that ended.
        let data = NativeTouchData(finger_event(0, 1.0, 2.0));
        assert!(data.touches().is_empty());
        assert_eq!(data.touches_changed().len(), 1);
    }

    #[test]
    fn test_synthetic_focus_and_input_events() {
        use crate::dioxus_document::DioxusDocument;
        use blitz_dom::{Document, DocumentConfig};
        use dioxus::prelude::*;
        use std::cell::RefCell;

        #[derive(Clone, PartialEq)]
        struct TestProps {
            focused: Rc<RefCell<bool>>,
            typed: Rc<RefCell<String>>,
        }

        fn test_app(props: TestProps) -> Element {
            rsx! {
                input {
                    id: "test-input",
                    onfocus: move |_| {
                        *props.focused.borrow_mut() = true;
                    },
                    oninput: move |evt: FormEvent| {
                        *props.typed.borrow_mut() = evt.value();
                    },
                }
            }
        }

        let focused_signal = Rc::new(RefCell::new(false));
        let typed_signal = Rc::new(RefCell::new(String::new()));

        let props = TestProps {
            focused: focused_signal.clone(),
            typed: typed_signal.clone(),
        };

        let vdom = VirtualDom::new_with_props(test_app, props);
        let mut doc = DioxusDocument::new(vdom, DocumentConfig::default());
        doc.initial_build();

        let input_id = doc.inner.borrow().get_element_by_id("test-input").unwrap();

        // Establish the Dioxus runtime guard for the test thread
        let _guard = dioxus_core::RuntimeGuard::new(doc.vdom.runtime());

        // 1. Dispatch synthetic focus
        let focused = dispatch_synthetic_focus(&mut doc.inner.borrow_mut(), input_id);
        assert!(focused, "dispatch_synthetic_focus must return true");
        doc.poll(None);
        assert!(*focused_signal.borrow(), "onfocus handler must have fired and updated signal");
        assert_eq!(doc.inner.borrow().get_focussed_node_id(), Some(input_id));

        // 2. Dispatch synthetic input
        let input_dispatched = dispatch_synthetic_input(&mut doc.inner.borrow_mut(), input_id, "hello world");
        assert!(input_dispatched, "dispatch_synthetic_input must return true");
        doc.poll(None);
        assert_eq!(*typed_signal.borrow(), "hello world", "oninput handler must receive new value");
    }

    #[test]
    fn test_synthetic_keyboard_events() {
        use crate::dioxus_document::DioxusDocument;
        use blitz_dom::{Document, DocumentConfig};
        use dioxus::prelude::*;
        use std::cell::RefCell;

        #[derive(Clone, PartialEq)]
        struct KeyAppProps {
            input_focused: Rc<RefCell<bool>>,
            btn_focused: Rc<RefCell<bool>>,
            typed: Rc<RefCell<String>>,
            clicks: Rc<RefCell<u32>>,
        }

        fn key_test_app(props: KeyAppProps) -> Element {
            let in_foc_f = props.input_focused.clone();
            let in_foc_b = props.input_focused.clone();
            let typed_c = props.typed.clone();
            let btn_foc_f = props.btn_focused.clone();
            let btn_foc_b = props.btn_focused.clone();
            let clicks_c = props.clicks.clone();
            rsx! {
                div {
                    input {
                        id: "key-input",
                        onfocus: move |_| {
                            *in_foc_f.borrow_mut() = true;
                        },
                        onblur: move |_| {
                            *in_foc_b.borrow_mut() = false;
                        },
                        oninput: move |evt: FormEvent| {
                            *typed_c.borrow_mut() = evt.value();
                        },
                    }
                    button {
                        id: "key-button",
                        onfocus: move |_| {
                            *btn_foc_f.borrow_mut() = true;
                        },
                        onblur: move |_| {
                            *btn_foc_b.borrow_mut() = false;
                        },
                        onclick: move |_| {
                            *clicks_c.borrow_mut() += 1;
                        },
                        "Submit"
                    }
                }
            }
        }

        let input_focused = Rc::new(RefCell::new(false));
        let btn_focused = Rc::new(RefCell::new(false));
        let typed = Rc::new(RefCell::new(String::new()));
        let clicks = Rc::new(RefCell::new(0u32));

        let props = KeyAppProps {
            input_focused: input_focused.clone(),
            btn_focused: btn_focused.clone(),
            typed: typed.clone(),
            clicks: clicks.clone(),
        };

        let vdom = VirtualDom::new_with_props(key_test_app, props);
        let mut doc = DioxusDocument::new(vdom, DocumentConfig::default());
        doc.initial_build();
        doc.inner.borrow_mut().resolve(0.0);

        let input_id = doc.inner.borrow().get_element_by_id("key-input").unwrap();
        let btn_id = doc.inner.borrow().get_element_by_id("key-button").unwrap();

        let _guard = dioxus_core::RuntimeGuard::new(doc.vdom.runtime());

        // 1. Focus traversal via Tab
        let tab1 = dispatch_synthetic_key(&mut doc.inner.borrow_mut(), None, "Tab", Modifiers::empty()).unwrap();
        assert_eq!(tab1, input_id, "Tab from initial state should focus input");
        doc.poll(None);
        assert!(*input_focused.borrow(), "Input onfocus must have run");
        assert!(!*btn_focused.borrow());

        // Tab again to focus button
        let tab2 = dispatch_synthetic_key(&mut doc.inner.borrow_mut(), None, "Tab", Modifiers::empty()).unwrap();
        assert_eq!(tab2, btn_id, "Tab from input should focus button");
        doc.poll(None);
        assert!(!*input_focused.borrow(), "Input onblur must have run");
        assert!(*btn_focused.borrow(), "Button onfocus must have run");

        // Shift+Tab back to input
        let stab = dispatch_synthetic_key(&mut doc.inner.borrow_mut(), None, "Shift+Tab", Modifiers::empty()).unwrap();
        assert_eq!(stab, input_id, "Shift+Tab should focus input again");
        doc.poll(None);
        assert!(*input_focused.borrow(), "Input onfocus must have run again");
        assert!(!*btn_focused.borrow(), "Button onblur must have run");

        // 2. Text editing via typing and Backspace
        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(input_id), "a", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*typed.borrow(), "a");

        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(input_id), "c", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*typed.borrow(), "ac");

        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(input_id), "Backspace", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*typed.borrow(), "a");

        // 3. Modifier sentinel: Ctrl/Cmd + A (Select All) then overwrite
        #[cfg(target_os = "macos")]
        let action_mod = Modifiers::SUPER;
        #[cfg(not(target_os = "macos"))]
        let action_mod = Modifiers::CONTROL;

        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(input_id), "a", action_mod).unwrap();
        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(input_id), "z", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*typed.borrow(), "z", "Cmd/Ctrl + A followed by typing 'z' must replace entire text");

        // 4. Button activation via Enter and Space
        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), None, "Tab", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(doc.inner.borrow().get_focussed_node_id(), Some(btn_id));

        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(btn_id), "Enter", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*clicks.borrow(), 1, "Enter on button must trigger click");

        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), Some(btn_id), "Space", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(*clicks.borrow(), 2, "Space on button must trigger click");

        // 5. Escape clears focus
        dispatch_synthetic_key(&mut doc.inner.borrow_mut(), None, "Escape", Modifiers::empty()).unwrap();
        doc.poll(None);
        assert_eq!(doc.inner.borrow().active_focus_node_id(), None, "Escape must clear explicit focus");
        assert!(!*btn_focused.borrow(), "Button onblur must have run on Escape");
    }
}
