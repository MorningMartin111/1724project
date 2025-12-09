use futures::TryStreamExt;
use gloo_net::eventsource::futures::EventSource;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

// --- 数据结构 ---

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Message {
    id: String,
    role: String,
    content: String,
}

#[derive(Clone, PartialEq)]
struct Session {
    id: String,
    title: String,
    messages: Vec<Message>,
}

fn create_new_session_struct() -> Session {
    Session {
        id: Uuid::new_v4().to_string(),
        title: "New Chat".to_string(),
        messages: Vec::new(),
    }
}

// --- 主组件 ---

#[function_component(App)]
fn app() -> Html {
    let first_session = create_new_session_struct();
    let first_id = first_session.id.clone();

    let sessions = use_state(|| vec![first_session]);
    let current_session_id = use_state(|| first_id);
    let input_value = use_state(|| String::new());
    let is_loading = use_state(|| false);

    let current_session = {
        let sessions_list = (*sessions).clone();
        let current_id = (*current_session_id).clone();
        sessions_list.into_iter()
            .find(|s| s.id == current_id)
            .unwrap_or_else(create_new_session_struct)
    };

    // --- 事件处理 ---

    let on_new_chat = {
        let sessions = sessions.clone();
        let current_session_id = current_session_id.clone();
        Callback::from(move |_| {
            let new_session = create_new_session_struct();
            let mut new_list = (*sessions).clone();
            new_list.insert(0, new_session.clone());
            sessions.set(new_list);
            current_session_id.set(new_session.id);
        })
    };

    let on_select_session = {
        let current_session_id = current_session_id.clone();
        Callback::from(move |id: String| {
            current_session_id.set(id);
        })
    };

    let on_input = {
        let input_value = input_value.clone();
        Callback::from(move |e: InputEvent| {
            let input: HtmlTextAreaElement = e.target_unchecked_into();
            input_value.set(input.value());
        })
    };

    // --- 核心提交逻辑 ---
    let on_submit = {
        let input_value = input_value.clone();
        let sessions = sessions.clone();
        let current_session_id = current_session_id.clone();
        let is_loading = is_loading.clone();

        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let prompt = (*input_value).clone();
            if prompt.trim().is_empty() || *is_loading {
                return;
            }

            // 1. UI: 添加用户消息
            let mut current_sessions_list = (*sessions).clone();
            if let Some(session) = current_sessions_list.iter_mut().find(|s| s.id == *current_session_id) {
                if session.messages.is_empty() {
                    session.title = prompt.chars().take(20).collect();
                }
                session.messages.push(Message {
                    id: Uuid::new_v4().to_string(),
                    role: "user".to_string(),
                    content: prompt.clone(),
                });
                // 预留 AI 消息位
                session.messages.push(Message {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".to_string(),
                    content: String::new(),
                });
            }
            sessions.set(current_sessions_list);
            input_value.set(String::new());
            is_loading.set(true);

            // 2. 启动流式请求
            let sessions = sessions.clone();
            let current_session_id = current_session_id.clone();
            let is_loading = is_loading.clone();
            
            spawn_local(async move {
                let url = format!(
                    "http://localhost:8000/chat/stream?prompt={}&max_tokens=200", 
                    urlencoding::encode(&prompt)
                );
                
                web_sys::console::log_1(&format!("Connecting to: {}", url).into());

                // 创建 EventSource
                let mut es = EventSource::new(&url).unwrap();
                // 订阅 "message" 事件，这必须与后端 .event("message") 对应
                let mut stream = es.subscribe("message").unwrap();

                // 循环接收
                while let Ok(Some((_, event))) = stream.try_next().await {
                    if let Some(data) = event.data().as_string() {
                        // web_sys::console::log_1(&format!("Chunk: {}", data).into()); // 调试用
                        
                        let mut list = (*sessions).clone();
                        if let Some(session) = list.iter_mut().find(|s| s.id == *current_session_id) {
                            if let Some(last_msg) = session.messages.last_mut() {
                                last_msg.content.push_str(&data);
                            }
                        }
                        sessions.set(list);
                    }
                }
                
                web_sys::console::log_1(&"Stream finished".into());
                is_loading.set(false);
            });
        })
    };

    let on_keydown = {
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Enter" && !e.shift_key() {
                // 可以在这里触发提交，目前留空
            }
        })
    };

    // --- 视图渲染 ---

    let sidebar_list_view = sessions.iter().map(|session| {
        let id = session.id.clone();
        let is_active = session.id == *current_session_id;
        let bg = if is_active { "bg-gray-800" } else { "hover:bg-gray-900" };
        let on_click = on_select_session.clone();
        
        html! {
            <button 
                key={session.id.clone()}
                onclick={move |_| on_click.emit(id.clone())}
                class={format!("w-full flex items-center gap-3 px-3 py-3 text-sm text-gray-100 rounded-md transition-colors truncate {}", bg)}
            >
                <svg class="h-4 w-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path></svg>
                <span class="truncate">{&session.title}</span>
            </button>
        }
    }).collect::<Html>();

    let chat_messages_view = if current_session.messages.is_empty() {
        html! {
            <div class="flex flex-col items-center justify-center h-[50vh] text-gray-100">
                <div class="bg-gray-700 p-4 rounded-full mb-4">
                    <svg class="h-10 w-10" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path d="M12 2a10 10 0 1 0 10 10H12V2z"></path></svg>
                </div>
                <h2 class="text-2xl font-semibold">{"How can I help you today?"}</h2>
            </div>
        }
    } else {
        current_session.messages.iter().map(|msg| {
            let is_user = msg.role == "user";
            let bg = if is_user { "" } else { "bg-gray-700/30" };
            let icon_bg = if is_user { "bg-purple-600" } else { "bg-green-500" };
            let name = if is_user { "You" } else { "AI" };

            html! {
                <div key={msg.id.clone()} class={format!("w-full border-b border-black/10 dark:border-gray-900/50 text-gray-100 {}", bg)}>
                    <div class="max-w-3xl mx-auto flex gap-4 p-4 md:py-6 text-base">
                        <div class={format!("w-8 h-8 rounded-sm flex items-center justify-center flex-shrink-0 font-bold text-sm {}", icon_bg)}>
                            {name}
                        </div>
                        <div class="relative flex-1 overflow-hidden leading-7 whitespace-pre-wrap">
                            { &msg.content }
                        </div>
                    </div>
                </div>
            }
        }).collect::<Html>()
    };

    html! {
        <div class="flex h-screen bg-gray-900 text-gray-100 font-sans overflow-hidden">
            <div class="w-64 bg-black flex flex-col border-r border-gray-800 hidden md:flex">
                <div class="p-3">
                    <button 
                        onclick={on_new_chat}
                        class="flex items-center gap-3 w-full px-3 py-3 rounded-md border border-gray-700 hover:bg-gray-900 transition-colors text-sm text-white text-left"
                    >
                        <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg>
                        <span>{"New chat"}</span>
                    </button>
                </div>
                <div class="flex-1 overflow-y-auto px-3 py-2 space-y-2">
                    <div class="text-xs font-semibold text-gray-500 px-3 py-2">{"History"}</div>
                    { sidebar_list_view }
                </div>
                <div class="p-3 border-t border-gray-800">
                    <div class="flex items-center gap-3 px-3 py-3 hover:bg-gray-900 rounded-md cursor-pointer">
                        <div class="w-8 h-8 bg-green-600 rounded-sm flex items-center justify-center text-white font-bold">{"U"}</div>
                        <div class="text-sm font-bold">{"User"}</div>
                    </div>
                </div>
            </div>

            <div class="flex-1 flex flex-col h-full relative bg-gray-800">
                <div class="h-14 border-b border-gray-700/50 flex items-center justify-between px-4 bg-gray-800 text-gray-200">
                    <div class="font-medium">{"TinyLlama Chat"}</div>
                </div>

                <div class="flex-1 overflow-y-auto p-4 md:p-0">
                    <div class="flex flex-col pb-32">
                        { chat_messages_view }
                        {
                            if *is_loading {
                                html! {
                                    <div class="w-full bg-gray-700/30 border-b border-black/10 dark:border-gray-900/50 text-gray-100">
                                        <div class="max-w-3xl mx-auto flex gap-4 p-4 md:py-6">
                                            <div class="w-8 h-8 bg-green-500 rounded-sm flex items-center justify-center flex-shrink-0">
                                                <div class="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full"></div>
                                            </div>
                                            <div>{"Thinking..."}</div>
                                        </div>
                                    </div>
                                }
                            } else {
                                html! {}
                            }
                        }
                    </div>
                </div>

                <div class="absolute bottom-0 left-0 w-full bg-gradient-to-t from-gray-800 via-gray-800 to-transparent pt-10 pb-6">
                    <div class="max-w-3xl mx-auto px-4">
                        <form onsubmit={on_submit} class="relative flex items-center w-full p-3 bg-gray-700 rounded-xl border border-gray-600 shadow-xl">
                            <textarea 
                                value={(*input_value).clone()}
                                oninput={on_input}
                                onkeydown={on_keydown}
                                rows="1"
                                placeholder="Send a message..."
                                class="flex-1 bg-transparent border-0 focus:ring-0 resize-none outline-none text-white max-h-48 overflow-y-auto py-2 pl-2"
                                style="min-height: 24px;"
                            ></textarea>
                            <button 
                                type="submit"
                                disabled={*is_loading || input_value.trim().is_empty()}
                                class="p-2 rounded-md bg-green-600 text-white hover:bg-green-700 disabled:bg-gray-600 transition-colors ml-2"
                            >
                                <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><line x1="22" y1="2" x2="11" y2="13"></line><polygon points="22 2 15 22 11 13 2 9 22 2"></polygon></svg>
                            </button>
                        </form>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}