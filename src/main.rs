use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::accept_async;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use warp::Filter;
use futures_util::{SinkExt, StreamExt};
use webbrowser::open;
use local_ip_address::local_ip;

// 嵌入 index.html 文件内容
const INDEX_HTML: &str = include_str!("../web/index.html");

#[tokio::main]
async fn main() {
    let addr = "0.0.0.0:6699";
    let listener = TcpListener::bind(&addr).await.expect("Can't listen on port 6699");

    // 获取局域网 IP 地址
    let ip = local_ip().expect("Failed to get local IP address").to_string();
    println!("Local IP address: {}", ip);

    println!("Started server on port 6700.");
    println!("Press Ctrl-C to exit.");

    // 创建一个共享的客户端列表
    let clients: Arc<Mutex<HashMap<usize, mpsc::UnboundedSender<Message>>>> = Arc::new(Mutex::new(HashMap::new()));

    // WebSocket 服务
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let clients = clients.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, clients).await {
                    eprintln!("Error handling connection: {}", e);
                }
            });
        }
    });

    // 嵌入 index.html 
    let index_route = warp::path::end().map(move || {
        let html = INDEX_HTML.replace("{{ip}}", &ip);
        warp::reply::html(html)
    });

    // 提供静态文件服务
    let static_files = warp::path("static")
        .and(warp::fs::dir("web"));

    // 合并路由
    let routes = index_route.or(static_files);

    // 启动静态文件服务
    let server = warp::serve(routes)
        .run(([0, 0, 0, 0], 6700));

    // 打开默认浏览器
    open("http://127.0.0.1:6700").unwrap();

    // 启动服务器
    server.await;
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    clients: Arc<Mutex<HashMap<usize, mpsc::UnboundedSender<Message>>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ws_stream = accept_async(stream).await?;

    let (mut write, mut read) = ws_stream.split();

    // 创建一个通道来发送消息到客户端
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<Message>();

    // 将客户端添加到客户端列表
    let client_id = {
        let mut clients_lock = clients.lock().unwrap();
        let id = clients_lock.len();
        clients_lock.insert(id, client_tx);
        id
    };

    // 使用 tokio::select! 来同时处理读取和发送消息
    tokio::select! {
        _ = async {
            while let Some(msg_result) = read.next().await {
                match msg_result {
                    Ok(msg) => {
                        if msg.is_text() || msg.is_binary() {
                            // 将消息广播给所有客户端
                            let clients_lock = clients.lock().unwrap();
                            for (_, tx) in clients_lock.iter() {
                                if let Err(e) = tx.send(msg.clone()) {
                                    eprintln!("Error sending message to client: {}", e);
                                }
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("Error receiving message from client: {}", e);
                        break;
                    }
                }
            }
        } => {
            // 客户端断开连接时，从客户端列表中移除
            clients.lock().unwrap().remove(&client_id);
        },
        _ = async {
            while let Some(msg) = client_rx.recv().await {
                if let Err(e) = write.send(msg).await {
                    eprintln!("Error sending message to client: {}", e);
                    break;
                }
            }
        } => {
            // 客户端断开连接时，从客户端列表中移除
            clients.lock().unwrap().remove(&client_id);
        }
    }

    Ok(())
}

