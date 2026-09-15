use color::{palette::css::WHITE, parse_color};
use dioxus_native::prelude::*;
use dioxus_native::CustomWidgetAttr;
use std::any::Any;

use crate::{limits, Color, DemoMessage, DemoWidget, FEATURES, STYLES};

pub fn launch_dx_native() {
    let config: Vec<Box<dyn Any>> = vec![Box::new(FEATURES), Box::new(limits())];
    dioxus_native::launch_cfg(app, Vec::new(), config);
}

fn app() -> Element {
    let mut show_cube = use_signal(|| true);

    // 1. 기존 큐브 색상용 시그널
    let color_str = use_signal(|| String::from("red"));
    let color = use_memo(move || {
        parse_color(&color_str())
            .map(|c| c.to_alpha_color())
            .unwrap_or(WHITE)
            .split()
            .0
    });

    // 2. [추가] 한글 IME 테스트용 텍스트 시그널
    let mut test_text = use_signal(|| String::from("hangul"));

    use_effect(move || println!("{:?}", color().components));

    rsx!(
        style { {STYLES} }
        div { id: "overlay",
            h2 { "Control Panel" }
            button { onclick: move |_| *show_cube.write() = !show_cube(),
                if show_cube() {
                    "Hide cube"
                } else {
                    "Show cube"
                }
            }
            br {}
            ColorControl { label: "Color:", color_str }
            br {}

            // ==========================================
            // [추가] 한글 IME 테스트용 입력 필드 섹션
            // ==========================================
            div { style: "margin-top: 10px; display: flex; flex-direction: column; gap: 4px;",
                label { style: "font-weight: bold; font-size: 0.9rem;", "한글 IME 테스트 입력:" }
                input {
                    style: "
                        width: 100%;
                        padding: 8px;
                        font-size: 1rem;
                        color: black;
                        background: white;
                        border-radius: 4px;
                        border: 1px solid #ccc;
                        outline: none;
                    ",
                    placeholder: "여기에 한글을 입력해보세요...",
                    value: test_text(),
                    oninput: move |evt| { *test_text.write() = evt.value() },
                }
                p { style: "margin: 4px 0; color: #89b4fa; font-size: 0.95rem; word-break: break-all;",
                    "실시간 반영: {test_text}"
                }
            }
            // ==========================================

            p {
                "This overlay demonstrates that the custom WGPU content can be rendered beneath layers of HTML content"
            }
        }
        div { id: "underlay",
            h2 { "Underlay" }
            p {
                "This underlay demonstrates that the custom WGPU content can be rendered above layers and blended with the content underneath"
            }
        }
        header {
            h2 { "Blitz WGPU Demo" }
        }
        if show_cube() {
            SpinningCube { color }
        }
    )
}

#[component]
fn ColorControl(label: &'static str, color_str: Signal<String>) -> Element {
    rsx!(
        div { class: "color-control",
            {label}
            input {
                value: color_str(),
                oninput: move |evt| { *color_str.write() = evt.value() },
            }
        }
    )
}

#[component]
fn SpinningCube(color: Memo<Color>) -> Element {
    let (sender, demo_widget_attr) = use_hook(|| {
        let demo_widget = DemoWidget::new();
        let sender = demo_widget.sender();
        let attr = CustomWidgetAttr::new(demo_widget);
        (sender, attr)
    });

    use_effect(move || {
        sender.send(DemoMessage::SetColor(color())).unwrap();
    });

    rsx!(
        div { id: "canvas-container",
            object { "data": demo_widget_attr }
        }
    )
}
