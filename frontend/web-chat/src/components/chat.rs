use web_sys::{Element, ScrollBehavior, ScrollIntoViewOptions};
use yew::prelude::*;

use crate::components::collaborate::CollaborateResultView;
use crate::components::input::ChatInput;
use crate::components::message::{MessageBubble, TypingIndicator};
use crate::markdown;
use crate::types::{AgentMeta, ChatMessage};

#[derive(Properties, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub agents: Vec<AgentMeta>,
    pub is_loading: bool,
    #[prop_or_default]
    pub streaming_status: Option<String>,
    #[prop_or_default]
    pub streaming_text: String,
    pub on_send: Callback<String>,
    pub on_example_click: Callback<String>,
    pub has_agent: bool,
}

#[function_component(ChatArea)]
pub fn chat_area(props: &ChatProps) -> Html {
    let messages_end_ref = use_node_ref();

    // Auto-scroll to bottom when messages or streaming text changes
    {
        let messages_end_ref = messages_end_ref.clone();
        let msg_count = props.messages.len();
        let is_loading = props.is_loading;
        let has_streaming = !props.streaming_text.is_empty();
        use_effect_with((msg_count, is_loading, has_streaming), move |_| {
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
        // Streaming bubble: shown while text is arriving
        let streaming_bubble = if !props.streaming_text.is_empty() {
            let rendered = markdown::markdown_to_html(&props.streaming_text);
            Html::from_html_unchecked(AttrValue::from(format!(
                "<div class=\"message-row assistant streaming-bubble\">\
                    <div class=\"message-avatar\">AI</div>\
                    <div class=\"message-bubble md-content streaming\">{}</div>\
                </div>",
                rendered
            )))
        } else {
            html! {}
        };

        // Typing indicator: only shown when loading but no text yet
        let loading_indicator = if props.is_loading && props.streaming_text.is_empty() {
            html! { <TypingIndicator status={props.streaming_status.clone()} /> }
        } else if props.is_loading && !props.streaming_text.is_empty() {
            // Show compact status badge while text is streaming
            if let Some(ref status) = props.streaming_status {
                Html::from_html_unchecked(AttrValue::from(format!(
                    "<div class=\"streaming-status-badge\">{}</div>",
                    status
                )))
            } else {
                html! {}
            }
        } else {
            html! {}
        };

        html! {
            <>
                <div class="messages">
                    { for props.messages.iter().map(|msg| {
                        if let Some(ref result) = msg.collaboration_result {
                            html! {
                                <CollaborateResultView
                                    result={result.clone()}
                                    agents={props.agents.clone()}
                                />
                            }
                        } else {
                            html! { <MessageBubble message={msg.clone()} /> }
                        }
                    })}
                    { streaming_bubble }
                    { loading_indicator }
                    <div ref={messages_end_ref}></div>
                </div>
                <ChatInput on_send={props.on_send.clone()} disabled={!props.has_agent || props.is_loading} />
            </>
        }
    }
}
