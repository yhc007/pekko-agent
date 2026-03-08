use yew::prelude::*;
use wasm_bindgen::prelude::*;

use crate::markdown;
use crate::types::{ChatMessage, MessageRole};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window)]
    fn downloadTableAsCSV(message_id: &str);
}

#[derive(Properties, PartialEq)]
pub struct MessageProps {
    pub message: ChatMessage,
}

#[function_component(MessageBubble)]
pub fn message_bubble(props: &MessageProps) -> Html {
    let msg = &props.message;
    let role_class = match msg.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "assistant",
    };

    let avatar = match msg.role {
        MessageRole::User => "👤",
        MessageRole::Assistant => "AI",
        MessageRole::System => "AI",
    };

    let tools_html = if !msg.tools_used.is_empty() {
        html! {
            <div class="tools-used">
                { for msg.tools_used.iter().map(|t| html! {
                    <span class="tool-badge">{ format!("🔧 {t}") }</span>
                })}
            </div>
        }
    } else {
        html! {}
    };

    let token_html = if let Some(ref usage) = msg.token_usage {
        html! {
            <div class="token-info">
                { format!("토큰: {} → {}", usage.input_tokens, usage.output_tokens) }
            </div>
        }
    } else {
        html! {}
    };

    // 테이블이 있는지 확인 (마크다운 테이블 형식: |로 시작하는 줄)
    let has_table = msg.content.lines().any(|line| line.trim().starts_with('|'));
    let msg_id = msg.id.clone();
    
    // 어시스턴트/시스템 메시지는 마크다운 렌더링, 사용자 메시지는 일반 텍스트
    let content_html = match msg.role {
        MessageRole::Assistant | MessageRole::System => {
            let rendered = markdown::markdown_to_html(&msg.content);
            Html::from_html_unchecked(AttrValue::from(
                format!("<div class=\"message-bubble md-content\" id=\"msg-{}\">{}</div>", msg.id, rendered)
            ))
        }
        MessageRole::User => {
            html! { <div class="message-bubble">{ &msg.content }</div> }
        }
    };
    
    // 테이블이 있으면 다운로드 버튼 표시
    let download_btn = if has_table && matches!(msg.role, MessageRole::Assistant | MessageRole::System) {
        let id = msg_id.clone();
        html! {
            <button class="download-csv-btn" onclick={Callback::from(move |_| {
                downloadTableAsCSV(&id);
            })}>
                { "📥 엑셀 다운로드" }
            </button>
        }
    } else {
        html! {}
    };

    html! {
        <div class={classes!("message-row", role_class)}>
            <div class="message-avatar">{ avatar }</div>
            <div>
                { content_html }
                { download_btn }
                { tools_html }
                { token_html }
            </div>
        </div>
    }
}

#[function_component(TypingIndicator)]
pub fn typing_indicator() -> Html {
    html! {
        <div class="message-row assistant">
            <div class="message-avatar">{ "AI" }</div>
            <div class="typing-indicator">
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
            </div>
        </div>
    }
}
