use yew::prelude::*;

use crate::markdown;
use crate::types::{ChatMessage, MessageRole};

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
        MessageRole::Assistant => "🤖",
        MessageRole::System => "ℹ️",
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

    // 어시스턴트/시스템 메시지는 마크다운 렌더링, 사용자 메시지는 일반 텍스트
    let content_html = match msg.role {
        MessageRole::Assistant | MessageRole::System => {
            let rendered = markdown::markdown_to_html(&msg.content);
            Html::from_html_unchecked(AttrValue::from(
                format!("<div class=\"message-bubble md-content\">{rendered}</div>")
            ))
        }
        MessageRole::User => {
            html! { <div class="message-bubble">{ &msg.content }</div> }
        }
    };

    html! {
        <div class={classes!("message-row", role_class)}>
            <div class="message-avatar">{ avatar }</div>
            <div>
                { content_html }
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
            <div class="message-avatar">{ "🤖" }</div>
            <div class="typing-indicator">
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
            </div>
        </div>
    }
}
