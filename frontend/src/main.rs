// use futures::TryStreamExt;
// use gloo_net::eventsource::futures::EventSource;
// use serde::{Deserialize, Serialize};
// use uuid::Uuid;
// use wasm_bindgen_futures::spawn_local;
// use web_sys::{HtmlTextAreaElement, HtmlSelectElement}; // 引入 HtmlSelectElement
// use yew::prelude::*;

// // --- 数据结构 ---

// #[derive(Clone, PartialEq, Serialize, Deserialize)]
// struct Message {
//     id: String,
//     role: String,
//     content: String,
// }

// #[derive(Clone, PartialEq)]
// struct Session {
//     id: String,
//     title: String,
//     messages: Vec<Message>,
// }

// fn create_new_session_struct() -> Session {
//     Session {
//         id: Uuid::new_v4().to_string(),
//         title: "New Chat".to_string(),
//         messages: Vec::new(),
//     }
// }

// // --- 主组件 ---

// #[function_component(App)]
// fn app() -> Html {
//     let first_session = create_new_session_struct();
//     let first_id = first_session.id.clone();

//     // 状态定义
//     let sessions = use_state(|| vec![first_session]);
//     let current_session_id = use_state(|| first_id);
//     let input_value = use_state(|| String::new());
//     let is_loading = use_state(|| false);
    
//     // 🔥 新增：用于存储当前选择的模型端口，默认为 "8000"
//     let selected_model_port = use_state(|| "8000".to_string());

//     let current_session = {
//         let sessions_list = (*sessions).clone();
//         let current_id = (*current_session_id).clone();
//         sessions_list.into_iter()
//             .find(|s| s.id == current_id)
//             .unwrap_or_else(create_new_session_struct)
//     };

//     // --- 事件处理 ---

//     // 1. 新建会话
//     let on_new_chat = {
//         let sessions = sessions.clone();
//         let current_session_id = current_session_id.clone();
//         Callback::from(move |_| {
//             let new_session = create_new_session_struct();
//             let mut new_list = (*sessions).clone();
//             new_list.insert(0, new_session.clone());
//             sessions.set(new_list);
//             current_session_id.set(new_session.id);
//         })
//     };

//     // 2. 切换会话
//     let on_select_session = {
//         let current_session_id = current_session_id.clone();
//         Callback::from(move |id: String| {
//             current_session_id.set(id);
//         })
//     };

//     // 3. 输入框输入
//     let on_input = {
//         let input_value = input_value.clone();
//         Callback::from(move |e: InputEvent| {
//             let input: HtmlTextAreaElement = e.target_unchecked_into();
//             input_value.set(input.value());
//         })
//     };

//     // 🔥 4. 模型切换事件
//     let on_model_change = {
//         let selected_model_port = selected_model_port.clone();
//         Callback::from(move |e: Event| {
//             let input: HtmlSelectElement = e.target_unchecked_into();
//             selected_model_port.set(input.value());
//         })
//     };

//     // 5. 提交发送
//     let on_submit = {
//         let input_value = input_value.clone();
//         let sessions = sessions.clone();
//         let current_session_id = current_session_id.clone();
//         let is_loading = is_loading.clone();
//         // 🔥 捕获当前选择的端口
//         let selected_model_port = selected_model_port.clone();

//         Callback::from(move |e: SubmitEvent| {
//             e.prevent_default();
//             let prompt = (*input_value).clone();
//             if prompt.trim().is_empty() || *is_loading {
//                 return;
//             }

//             // UI: 添加用户消息和空的 AI 消息占位
//             let mut current_sessions_list = (*sessions).clone();
//             if let Some(session) = current_sessions_list.iter_mut().find(|s| s.id == *current_session_id) {
//                 if session.messages.is_empty() {
//                     session.title = prompt.chars().take(20).collect();
//                 }
//                 session.messages.push(Message {
//                     id: Uuid::new_v4().to_string(),
//                     role: "user".to_string(),
//                     content: prompt.clone(),
//                 });
//                 session.messages.push(Message {
//                     id: Uuid::new_v4().to_string(),
//                     role: "assistant".to_string(),
//                     content: String::new(), // 此时是空的
//                 });
//             }
//             sessions.set(current_sessions_list.clone()); // 更新 UI
//             input_value.set(String::new());
//             is_loading.set(true);

//             // 启动流式请求
//             let sessions = sessions.clone();
//             let current_session_id = current_session_id.clone();
//             let is_loading = is_loading.clone();
//             let mut local_sessions_buffer = current_sessions_list; 
            
//             // 🔥 获取要使用的端口
//             let port = (*selected_model_port).clone();

//             spawn_local(async move {
//                 // 🔥 动态构建 URL
//                 let url = format!(
//                     "http://localhost:{}/chat/stream?prompt={}&max_tokens=200", 
//                     port,
//                     urlencoding::encode(&prompt)
//                 );
                
//                 web_sys::console::log_1(&format!("Connecting to: {}", url).into());

//                 let mut es = EventSource::new(&url).unwrap();
//                 let mut stream = es.subscribe("message").unwrap();

//                 while let Ok(Some((_, event))) = stream.try_next().await {
//                     if let Some(data) = event.data().as_string() {
//                         if let Some(session) = local_sessions_buffer.iter_mut().find(|s| s.id == *current_session_id) {
//                             if let Some(last_msg) = session.messages.last_mut() {
//                                 last_msg.content.push_str(&data);
//                             }
//                         }
//                         sessions.set(local_sessions_buffer.clone());
//                     }
//                 }
                
//                 web_sys::console::log_1(&"Stream finished".into());
//                 is_loading.set(false);
//             });
//         })
//     };

//     let on_keydown = {
//         Callback::from(move |e: KeyboardEvent| {
//             if e.key() == "Enter" && !e.shift_key() {
//                 // 这里可以留空，或者调用 prevent default
//             }
//         })
//     };

//     // --- 视图渲染 ---
//     let sidebar_list_view = sessions.iter().map(|session| {
//         let id = session.id.clone();
//         let is_active = session.id == *current_session_id;
//         let bg = if is_active { "bg-gray-800" } else { "hover:bg-gray-900" };
//         let on_click = on_select_session.clone();
        
//         html! {
//             <button 
//                 key={session.id.clone()}
//                 onclick={move |_| on_click.emit(id.clone())}
//                 class={format!("w-full flex items-center gap-3 px-3 py-3 text-sm text-gray-100 rounded-md transition-colors truncate {}", bg)}
//             >
//                 <svg class="h-4 w-4 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path></svg>
//                 <span class="truncate">{&session.title}</span>
//             </button>
//         }
//     }).collect::<Html>();

//     let chat_messages_view = if current_session.messages.is_empty() {
//         html! {
//             <div class="flex flex-col items-center justify-center h-[50vh] text-gray-100">
//                 <div class="bg-gray-700 p-4 rounded-full mb-4">
//                     <svg class="h-10 w-10" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path d="M12 2a10 10 0 1 0 10 10H12V2z"></path></svg>
//                 </div>
//                 <h2 class="text-2xl font-semibold">{"How can I help you today?"}</h2>
//             </div>
//         }
//     } else {
//         current_session.messages.iter().map(|msg| {
//             let is_user = msg.role == "user";
//             let bg = if is_user { "" } else { "bg-gray-700/30" };
//             let icon_bg = if is_user { "bg-purple-600" } else { "bg-green-500" };
//             let name = if is_user { "You" } else { "AI" };

//             html! {
//                 <div key={msg.id.clone()} class={format!("w-full border-b border-black/10 dark:border-gray-900/50 text-gray-100 {}", bg)}>
//                     <div class="max-w-3xl mx-auto flex gap-4 p-4 md:py-6 text-base">
//                         <div class={format!("w-8 h-8 rounded-sm flex items-center justify-center flex-shrink-0 font-bold text-sm {}", icon_bg)}>
//                             {name}
//                         </div>
//                         <div class="relative flex-1 overflow-hidden leading-7 whitespace-pre-wrap">
//                             { &msg.content }
//                         </div>
//                     </div>
//                 </div>
//             }
//         }).collect::<Html>()
//     };

//     html! {
//         <div class="flex h-screen bg-gray-900 text-gray-100 font-sans overflow-hidden">
//             <div class="w-64 bg-black flex flex-col border-r border-gray-800 hidden md:flex">
                
//                 // --- Sidebar 顶部 ---
//                 <div class="p-3 space-y-2">
//                     // 🔥 模型选择下拉菜单
//                     <div class="relative">
//                         <select 
//                             onchange={on_model_change}
//                             class="w-full bg-gray-900 border border-gray-700 text-gray-200 text-sm rounded-md focus:ring-green-500 focus:border-green-500 block p-2.5 appearance-none cursor-pointer"
//                         >
//                             <option value="8000" selected={*selected_model_port == "8000"}>{"Llama 2 (Port 8000)"}</option>
//                             <option value="8001" selected={*selected_model_port == "8001"}>{"Mistral (Port 8001)"}</option>
//                         </select>
//                         // 下拉箭头图标
//                         <div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400">
//                             <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path></svg>
//                         </div>
//                     </div>

//                     <button 
//                         onclick={on_new_chat}
//                         class="flex items-center gap-3 w-full px-3 py-3 rounded-md border border-gray-700 hover:bg-gray-900 transition-colors text-sm text-white text-left"
//                     >
//                         <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg>
//                         <span>{"New chat"}</span>
//                     </button>
//                 </div>

//                 <div class="flex-1 overflow-y-auto px-3 py-2 space-y-2">
//                     <div class="text-xs font-semibold text-gray-500 px-3 py-2">{"History"}</div>
//                     { sidebar_list_view }
//                 </div>
//                 <div class="p-3 border-t border-gray-800">
//                     <div class="flex items-center gap-3 px-3 py-3 hover:bg-gray-900 rounded-md cursor-pointer">
//                         <div class="w-8 h-8 bg-green-600 rounded-sm flex items-center justify-center text-white font-bold">{"U"}</div>
//                         <div class="text-sm font-bold">{"User"}</div>
//                     </div>
//                 </div>
//             </div>

//             <div class="flex-1 flex flex-col h-full relative bg-gray-800">
//                 <div class="h-14 border-b border-gray-700/50 flex items-center justify-between px-4 bg-gray-800 text-gray-200">
//                     <div class="font-medium">{"AI Chat"}</div>
//                 </div>

//                 <div class="flex-1 overflow-y-auto p-4 md:p-0">
//                     <div class="flex flex-col pb-32">
//                         { chat_messages_view }
//                         {
//                             if *is_loading {
//                                 html! {
//                                     <div class="w-full bg-gray-700/30 border-b border-black/10 dark:border-gray-900/50 text-gray-100">
//                                         <div class="max-w-3xl mx-auto flex gap-4 p-4 md:py-6">
//                                             <div class="w-8 h-8 bg-green-500 rounded-sm flex items-center justify-center flex-shrink-0">
//                                                 <div class="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full"></div>
//                                             </div>
//                                             <div>{"Thinking..."}</div>
//                                         </div>
//                                     </div>
//                                 }
//                             } else {
//                                 html! {}
//                             }
//                         }
//                     </div>
//                 </div>

//                 <div class="absolute bottom-0 left-0 w-full bg-gradient-to-t from-gray-800 via-gray-800 to-transparent pt-10 pb-6">
//                     <div class="max-w-3xl mx-auto px-4">
//                         <form onsubmit={on_submit} class="relative flex items-center w-full p-3 bg-gray-700 rounded-xl border border-gray-600 shadow-xl">
//                             <textarea 
//                                 value={(*input_value).clone()}
//                                 oninput={on_input}
//                                 onkeydown={on_keydown}
//                                 rows="1"
//                                 placeholder="Send a message..."
//                                 class="flex-1 bg-transparent border-0 focus:ring-0 resize-none outline-none text-white max-h-48 overflow-y-auto py-2 pl-2"
//                                 style="min-height: 24px;"
//                             ></textarea>
//                             <button 
//                                 type="submit"
//                                 disabled={*is_loading || input_value.trim().is_empty()}
//                                 class="p-2 rounded-md bg-green-600 text-white hover:bg-green-700 disabled:bg-gray-600 transition-colors ml-2"
//                             >
//                                 <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><line x1="22" y1="2" x2="11" y2="13"></line><polygon points="22 2 15 22 11 13 2 9 22 2"></polygon></svg>
//                             </button>
//                         </form>
//                     </div>
//                 </div>
//             </div>
//         </div>
//     }
// }

// fn main() {
//     yew::Renderer::<App>::new().render();
// }
use futures::channel::oneshot;
use futures::stream::StreamExt;
use futures::TryStreamExt;
use gloo_net::eventsource::futures::EventSource;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlTextAreaElement, HtmlSelectElement};
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

    // 状态定义
    let sessions = use_state(|| vec![first_session]);
    let current_session_id = use_state(|| first_id);
    let input_value = use_state(|| String::new());
    let is_loading = use_state(|| false);
    let selected_model_port = use_state(|| "8000".to_string());

    // 用于存储停止信号的发送端
    let abort_handle = use_mut_ref(|| None::<oneshot::Sender<()>>);

    let current_session = {
        let sessions_list = (*sessions).clone();
        let current_id = (*current_session_id).clone();
        sessions_list.into_iter()
            .find(|s| s.id == current_id)
            .unwrap_or_else(create_new_session_struct)
    };

    // --- 事件处理 ---

    // 1. 核心停止逻辑
    let stop_chat = {
        let is_loading = is_loading.clone();
        let abort_handle = abort_handle.clone();
        Callback::from(move |_: ()| {
            if let Some(sender) = abort_handle.borrow_mut().take() {
                let _ = sender.send(()); 
            }
            is_loading.set(false);
        })
    };

    // 2. 停止按钮点击事件
    let on_stop_click = {
        let stop_chat = stop_chat.clone();
        Callback::from(move |_: MouseEvent| {
            stop_chat.emit(());
        })
    };

    // 3. 新建会话
    let on_new_chat = {
        let sessions = sessions.clone();
        let current_session_id = current_session_id.clone();
        let stop_chat = stop_chat.clone();
        
        Callback::from(move |_| {
            stop_chat.emit(()); // 先停止当前

            let new_session = create_new_session_struct();
            let mut new_list = (*sessions).clone();
            new_list.insert(0, new_session.clone());
            sessions.set(new_list);
            current_session_id.set(new_session.id);
        })
    };

    // 4. 切换会话
    let on_select_session = {
        let current_session_id = current_session_id.clone();
        Callback::from(move |id: String| {
            current_session_id.set(id);
        })
    };

    // 5. 输入框输入
    let on_input = {
        let input_value = input_value.clone();
        Callback::from(move |e: InputEvent| {
            let input: HtmlTextAreaElement = e.target_unchecked_into();
            input_value.set(input.value());
        })
    };

    // 6. 模型切换
    let on_model_change = {
        let selected_model_port = selected_model_port.clone();
        Callback::from(move |e: Event| {
            let input: HtmlSelectElement = e.target_unchecked_into();
            selected_model_port.set(input.value());
        })
    };

    // 7. 提交发送 (这里修复了所有权问题)
    let on_submit = {
        let input_value = input_value.clone();
        let sessions = sessions.clone();
        let current_session_id = current_session_id.clone();
        let is_loading = is_loading.clone();
        let selected_model_port = selected_model_port.clone();
        let abort_handle = abort_handle.clone();

        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let prompt = (*input_value).clone();
            if prompt.trim().is_empty() || *is_loading {
                return;
            }

            // UI 更新
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
                session.messages.push(Message {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".to_string(),
                    content: String::new(),
                });
            }
            sessions.set(current_sessions_list.clone());
            input_value.set(String::new());
            is_loading.set(true);

            // 准备异步任务
            let sessions = sessions.clone();
            let current_session_id = current_session_id.clone();
            let is_loading = is_loading.clone();
            let mut local_sessions_buffer = current_sessions_list; 
            let port = (*selected_model_port).clone();

            // 设置停止信号
            let (tx, rx) = oneshot::channel();
            *abort_handle.borrow_mut() = Some(tx);

            // 🔥🔥 关键修复：在这里克隆 abort_handle 给异步任务使用 🔥🔥
            // 这样原来的 abort_handle 仍然保留在闭包环境中，供下次点击使用
            let abort_handle = abort_handle.clone();

            spawn_local(async move {
                let url = format!(
                    "http://localhost:{}/chat/stream?prompt={}&max_tokens=200", 
                    port,
                    urlencoding::encode(&prompt)
                );
                
                if let Ok(mut es) = EventSource::new(&url) {
                    if let Ok(stream) = es.subscribe("message") {
                        let mut stream = stream.take_until(rx);

                        while let Ok(Some((_, event))) = stream.try_next().await {
                            if let Some(data) = event.data().as_string() {
                                if let Some(session) = local_sessions_buffer.iter_mut().find(|s| s.id == *current_session_id) {
                                    if let Some(last_msg) = session.messages.last_mut() {
                                        last_msg.content.push_str(&data);
                                    }
                                }
                                sessions.set(local_sessions_buffer.clone());
                            }
                        }
                    }
                }
                
                is_loading.set(false);
                // 异步任务结束后清理
                *abort_handle.borrow_mut() = None;
            });
        })
    };

    let on_keydown = {
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Enter" && !e.shift_key() {
                // e.prevent_default(); 
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
                <div class="p-3 space-y-2">
                    <div class="relative">
                        <select 
                            onchange={on_model_change}
                            class="w-full bg-gray-900 border border-gray-700 text-gray-200 text-sm rounded-md focus:ring-green-500 focus:border-green-500 block p-2.5 appearance-none cursor-pointer"
                        >
                            <option value="8000" selected={*selected_model_port == "8000"}>{"Llama 2 (Port 8000)"}</option>
                            <option value="8001" selected={*selected_model_port == "8001"}>{"Mistral (Port 8001)"}</option>
                        </select>
                        <div class="pointer-events-none absolute inset-y-0 right-0 flex items-center px-2 text-gray-400">
                            <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path></svg>
                        </div>
                    </div>

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
                    <div class="font-medium">{"AI Chat"}</div>
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
                            
                            {
                                if *is_loading {
                                    html! {
                                        <button 
                                            type="button"
                                            onclick={on_stop_click}
                                            class="p-2 rounded-md bg-red-600 text-white hover:bg-red-700 transition-colors ml-2"
                                            title="Stop generating"
                                        >
                                            <svg class="h-4 w-4" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12"></rect></svg>
                                        </button>
                                    }
                                } else {
                                    html! {
                                        <button 
                                            type="submit"
                                            disabled={input_value.trim().is_empty()}
                                            class="p-2 rounded-md bg-green-600 text-white hover:bg-green-700 disabled:bg-gray-600 transition-colors ml-2"
                                        >
                                            <svg class="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><line x1="22" y1="2" x2="11" y2="13"></line><polygon points="22 2 15 22 11 13 2 9 22 2"></polygon></svg>
                                        </button>
                                    }
                                }
                            }
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