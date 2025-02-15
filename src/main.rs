use axum::{
    body::Body,
    extract::{Multipart, Path},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use serde_json::json;
use sqlx::{query, SqlitePool};
use tokio::fs::{self, File};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: SqlitePool,
}

#[tokio::main]
async fn main() {
    let db = SqlitePool::connect("sqlite:filesdb.sqlite3").await.unwrap();
    query!(
        "CREATE TABLE IF NOT EXISTS files (id TEXT PRIMARY KEY, filename TEXT, upload_time TEXT)"
    )
    .execute(&db)
    .await
    .unwrap();

    let state = AppState { db };
    let config_txt = tokio::fs::read_to_string("config.json")
        .await
        .unwrap_or("{}".into());
    let config: serde_json::Value = serde_json::from_str(&config_txt).unwrap();

    let user = config["user"].as_str().unwrap_or("user");
    let password = config["password"].as_str().unwrap_or("password");
    let addr = config["address"].as_str().unwrap_or("127.0.0.1:7002");

    let app = Router::new()
        .route("/", get(homepage))
        .route("/files", post(upload_file).get(list_files))
        .route("/files/{fileid}", get(get_file).delete(delete_file))
        .layer(axum::extract::Extension(state))
        .layer(tower_http::validate_request::ValidateRequestHeaderLayer::basic(user, password));

    println!("Listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn homepage() -> Html<String> {
    let content = tokio::fs::read_to_string("src/main.html").await.unwrap();
    Html(content)
}

async fn upload_file(
    axum::extract::Extension(state): axum::extract::Extension<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    while let Some(field) = multipart.next_field().await.unwrap() {
        let filename = field.file_name().unwrap().to_string();
        let contents = field.bytes().await.unwrap();
        let id = Uuid::new_v4().to_string();
        let upload_time = Utc::now().to_rfc3339();
        let file_path = format!("uploads/{}", id);
        println!("Uploading file: '{}' of len: {}", file_path, contents.len());

        fs::write(&file_path, &contents).await.unwrap();
        query!(
            "INSERT INTO files (id, filename, upload_time) VALUES (?, ?, ?)",
            id,
            filename,
            upload_time
        )
        .execute(&state.db)
        .await
        .unwrap();
    }
    "Uploaded"
}

async fn list_files(
    axum::extract::Extension(state): axum::extract::Extension<AppState>,
) -> impl IntoResponse {
    let rows = query!("SELECT id, upload_time, filename FROM files")
        .fetch_all(&state.db)
        .await
        .unwrap();
    let files: Vec<_> = rows
        .iter()
        .map(|r| json!({"id": &r.id, "upload_time": &r.upload_time, "filename": &r.filename}))
        .collect();
    axum::Json(files)
}

async fn get_file(
    axum::extract::Extension(state): axum::extract::Extension<AppState>,
    Path(fileid): Path<String>,
) -> impl IntoResponse {
    let row = query!("SELECT filename FROM files WHERE id = ?", fileid)
        .fetch_optional(&state.db)
        .await
        .unwrap();
    if row.is_none() {
        return axum::http::Response::builder()
            .status(404)
            .body("No such file in DB".into())
            .unwrap();
    }

    let file_path = format!("uploads/{}", fileid);
    if std::path::Path::new(&file_path).exists() {
        let file = File::open(file_path).await.unwrap();
        let stream = tokio_util::io::ReaderStream::new(file);

        return axum::response::Response::builder()
            .header(
                "Content-Disposition",
                format!("inline; filename=\"{}\"", row.unwrap().filename.unwrap()),
            )
            .body(Body::from_stream(stream))
            .unwrap();
    }

    return axum::http::Response::builder()
        .status(500)
        .body("File not found".into())
        .unwrap();
}

async fn delete_file(
    axum::extract::Extension(state): axum::extract::Extension<AppState>,
    Path(fileid): Path<String>,
) -> impl IntoResponse {
    let result = query!("DELETE FROM files WHERE id = ?", fileid)
        .execute(&state.db)
        .await
        .unwrap();
    if result.rows_affected() > 0 {
        let file_path = format!("uploads/{}", fileid);
        let _ = fs::remove_file(file_path).await;
        "Deleted"
    } else {
        "File not found"
    }
}
