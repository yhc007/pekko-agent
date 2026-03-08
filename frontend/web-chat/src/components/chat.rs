use web_sys::{Element, ScrollBehavior, ScrollIntoViewOptions};
use yew::prelude::*;

use crate::components::input::ChatInput;
use crate::components::message::{MessageBubble, TypingIndicator};
use crate::types::ChatMessage;

#[derive(Properties, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub is_loading: bool,
    pub on_send: Callback<String>,
    pub on_example_click: Callback<String>,
    pub has_agent: bool,
}

#[function_component(ChatArea)]
pub fn chat_area(props: &ChatProps) -> Html {
    let messages_end_ref = use_node_ref();

    // Auto-scroll to bottom when new messages arrive
    {
        let messages_end_ref = messages_end_ref.clone();
        let msg_count = props.messages.len();
        let is_loading = props.is_loading;
        use_effect_with((msg_count, is_loading), move |_| {
            if let Some(el) = messages_end_ref.cast::<Element>() {
                let opts = ScrollIntoViewOptions::new();
                opts.set_behavior(ScrollBehavior::Smooth);
                el.scroll_into_view_with_scroll_into_view_options(&opts);
            }
            || ()
        });
    }

    let show_welcome = props.messages.is_empty() && !props.is_loading;

    if show_welcome {
        let examples = vec![
            "현재 진행 중인 위험작업 허가 현황을 알려줘",
            "이번 달 안전점검 일정은?",
            "MSDS 등록된 화학물질 목록을 보여줘",
            "최근 아차사고 발생 현황을 분석해줘",
        ];

        html! {
            <div class="welcome">
                <div class="welcome-icon"></div>
                <h2>{ "EHS AI Agent에 오신 것을 환영합니다" }</h2>
                <p>
                    if props.has_agent {
                        { "아래 예시를 클릭하거나 직접 질문해 보세요." }
                    } else {
                        { "좌측에서 에이전트를 선택한 후 질문해 보세요." }
                    }
                </p>
                if props.has_agent {
                    <div class="welcome-examples">
                        { for examples.iter().map(|ex| {
                            let text = ex.to_string();
                            let cb = props.on_example_click.clone();
                            let t = text.clone();
                            html! {
                                <div class="welcome-example"
                                    onclick={Callback::from(move |_: MouseEvent| cb.emit(t.clone()))}
                                >
                                    { format!("💬 {text}") }
                                </div>
                            }
                        })}
                    </div>
                }
                <ChatInput on_send={props.on_send.clone()} disabled={!props.has_agent || props.is_loading} />
            </div>
        }
    } else {
        html! {
            <>
                <div class="messages">
                    { for props.messages.iter().map(|msg| html! {
                        <MessageBubble message={msg.clone()} />
                    })}
                    if props.is_loading {
                        <TypingIndicator />
                    }
                    <div ref={messages_end_ref}></div>
                </div>
                <ChatInput on_send={props.on_send.clone()} disabled={!props.has_agent || props.is_loading} />
            </>
        }
    }
}
