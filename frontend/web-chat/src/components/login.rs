use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::api;
use crate::types::AuthResponse;

#[derive(Properties, PartialEq)]
pub struct LoginProps {
    pub on_login: Callback<AuthResponse>,
}

#[function_component(LoginScreen)]
pub fn login_screen(props: &LoginProps) -> Html {
    let api_key = use_state(String::new);
    let loading = use_state(|| false);
    let error = use_state(|| Option::<String>::None);

    let on_input = {
        let api_key = api_key.clone();
        Callback::from(move |e: InputEvent| {
            if let Some(target) = e.target() {
                if let Ok(input) = target.dyn_into::<HtmlInputElement>() {
                    api_key.set(input.value());
                }
            }
        })
    };

    let on_submit = {
        let api_key = api_key.clone();
        let loading = loading.clone();
        let error = error.clone();
        let on_login = props.on_login.clone();
        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let key = (*api_key).trim().to_string();
            if key.is_empty() {
                return;
            }
            loading.set(true);
            error.set(None);
            let loading2 = loading.clone();
            let error2 = error.clone();
            let on_login2 = on_login.clone();
            spawn_local(async move {
                match api::issue_token(&key).await {
                    Ok(auth) => {
                        loading2.set(false);
                        on_login2.emit(auth);
                    }
                    Err(e) => {
                        loading2.set(false);
                        error2.set(Some(e));
                    }
                }
            });
        })
    };

    html! {
        <div class="login-screen">
            <div class="login-card">
                <div class="login-logo"></div>
                <h2>{ "Pekko Agent" }</h2>
                <p>{ "EHS AI 에이전트에 접속하려면 API 키를 입력하세요." }</p>
                <form onsubmit={on_submit}>
                    <div class="login-field">
                        <label>{ "API 키" }</label>
                        <input
                            type="password"
                            placeholder="pekko_admin_... 또는 pekko_agent_..."
                            value={(*api_key).clone()}
                            oninput={on_input}
                            disabled={*loading}
                            autocomplete="off"
                        />
                    </div>
                    <button class="login-btn" type="submit" disabled={*loading}>
                        if *loading { { "인증 중..." } } else { { "접속하기" } }
                    </button>
                </form>
                if let Some(ref err) = *error {
                    <div class="login-error">{ format!("인증 실패: {}", err) }</div>
                }
            </div>
        </div>
    }
}
