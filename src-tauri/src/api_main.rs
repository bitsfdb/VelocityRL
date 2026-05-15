mod engine;

use axum::{
    routing::{get, post},
    Json, Router,
    extract::Query,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use crate::engine::psynet::PsynetClient;

#[derive(Deserialize)]
struct FetchParams {
    token: String,
    account: String,
}

mod csv_converter;

struct AppState {
    items: Mutex<Value>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    
    let items_val = if std::path::Path::new("../items.csv").exists() {
        csv_converter::convert_csv_to_json("../items.csv").unwrap_or(json!({"Items": []}))
    } else {
        json!({"Items": []})
    };

    let state = Arc::new(AppState {
        items: Mutex::new(items_val),
    });

    let app = Router::new()
        .route("/items.json", get(handle_get_items))
        .route("/fetch", post(handle_fetch_catalog))
        .route("/inventory", post(handle_get_inventory))
        .route("/trade-in", post(handle_trade_in))
        .route("/drops", get(handle_get_drops))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("VelocityRL API Server running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_get_items(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<Value> {
    let items = state.items.lock().unwrap();
    Json(items.clone())
}

async fn handle_fetch_catalog(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(params): Json<FetchParams>,
) -> Result<Json<Value>, String> {
    use crate::engine::psynet::{EpicAuth, EPIC_LAUNCHER_CLIENT_ID};

    // If the token looks like an exchange code, exchange it for a ticket
    let ticket = if params.token.len() < 64 {
        let epic = EpicAuth::new(EPIC_LAUNCHER_CLIENT_ID);
        let auth = epic.exchange_code(&params.token).await?;
        auth["access_token"].as_str().ok_or("No access token in Epic response")?.to_string()
    } else {
        params.token
    };

    let mut client = PsynetClient::new(ticket);
    client.login(&params.account).await.map_err(|e| e.to_string())?;
    
    let products = client.get_all_products().await.map_err(|e| e.to_string())?;
    
    let mut new_items = Vec::new();
    for p in products {
        new_items.push(json!({
            "ID": p["ProductID"],
            "Product": p["Label"].as_str().unwrap_or("Unknown"),
            "Quality": p["Quality"].as_str().unwrap_or("Common"),
            "Slot": p["Slot"].as_str().unwrap_or("Unknown"),
            "AssetPackage": "", 
            "AssetPath": "",
            "image_url": p["Thumbnail"].as_str().unwrap_or("")
        }));
    }

    let mut items_data = state.items.lock().unwrap();
    *items_data = json!({ "Items": new_items });

    Ok(Json(items_data.clone()))
}

async fn handle_get_inventory(
    Json(params): Json<FetchParams>,
) -> Result<Json<Value>, String> {
    let mut client = PsynetClient::new(params.token);
    client.login(&params.account).await.map_err(|e| e.to_string())?;
    let products = client.get_player_products(0).await.map_err(|e| e.to_string())?;
    Ok(Json(json!({ "Inventory": products })))
}

async fn handle_trade_in(
    Json(body): Json<Value>,
) -> Result<Json<Value>, String> {
    let token = body["token"].as_str().ok_or("No token")?;
    let account = body["account"].as_str().ok_or("No account")?;
    let instances = body["instances"].as_array().ok_or("No instances")?
        .iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();

    let mut client = PsynetClient::new(token.to_string());
    client.login(account).await.map_err(|e| e.to_string())?;
    let drops = client.trade_in(instances).await.map_err(|e| e.to_string())?;
    Ok(Json(json!({ "Drops": drops })))
}

async fn handle_get_drops() -> Result<Json<Value>, String> {
    // This requires a client, but we'll use a generic one or cache it
    // For now, return a placeholder or allow passing token
    Ok(Json(json!({ "status": "Requires session" })))
}
