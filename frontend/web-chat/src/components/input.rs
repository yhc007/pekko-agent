use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, HtmlTextAreaElement, KeyboardEvent};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct InputProps {
    pub on_send: Callback<String>,
    pub disabled: bool,
}

#[function_component(ChatInput)]
pub fn chat_input(props: &InputProps) -> Html {
    let input_value = use_state(String::new);
    let textarea_ref = use_node_ref();

    let on_input = {
        let input_value = input_value.clone();
        Callback::from(move |e: InputEvent| {
            if let Some(target) = e.target() {
                if let Ok(textarea) = target.dyn_into::<HtmlElement>() {
                    if let Ok(ta) = textarea.clone().dyn_into::<HtmlTextAreaElement>() {
                        input_value.set(ta.value());
                    }
                    // Auto-resize
                    textarea.style().set_property("height", "auto").ok();
                    let scroll_h = textarea.scroll_height();
                    let h = scroll_h.min(120);
                    textarea
                        .style()
                        .set_property("height", &format!("{h}px"))
                        .ok();
                }
            }
        })
    };

    let on_keydown = {
        let input_value = input_value.clone();
        let on_send = props.on_send.clone();
        let disabled = props.disabled;
        let textarea_ref = textarea_ref.clone();
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Enter" && !e.shift_key() && !disabled {
                e.prevent_default();
                let val = (*input_value).trim().to_string();
                if !val.is_empty() {
                    on_send.emit(val);
                    input_value.set(String::new());
                    // Reset height
                    if let Some(el) = textarea_ref.cast::<HtmlTextAreaElement>() {
                        el.set_value("");
                    }
                    if let Some(el) = textarea_ref.cast::<HtmlElement>() {
                        el.style().set_property("height", "auto").ok();
                    }
                }
            }
        })
    };

    let on_click_send = {
        let input_value = input_value.clone();
        let on_send = props.on_send.clone();
        let textarea_ref = textarea_ref.clone();
        Callback::from(move |_: MouseEvent| {
            let val = (*input_value).trim().to_string();
            if !val.is_empty() {
                on_send.emit(val);
                input_value.set(String::new());
                if let Some(el) = textarea_ref.cast::<HtmlTextAreaElement>() {
                    el.set_value("");
                }
                if let Some(el) = textarea_ref.cast::<HtmlElement>() {
                    el.style().set_property("height", "auto").ok();
                }
            }
        })
    };

    let can_send = !(*input_value).trim().is_empty() && !props.disabled;

    html! {
        <div class="input-area">
            <div class="input-wrapper">
                <textarea
                    ref={textarea_ref}
                    rows="1"
                    placeholder="메시지를 입력하세요..."
                    value={(*input_value).clone()}
                    oninput={on_input}
                    onkeydown={on_keydown}
                    disabled={props.disabled}
                />
                <button
                    class="send-btn"
                    onclick={on_click_send}
                    disabled={!can_send}
                >
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z"/>
                    </svg>
                </button>
            </div>
        </div>
    }
}
