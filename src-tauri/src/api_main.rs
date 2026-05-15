mod engine;

use axum::{
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/items.json", get(handle_get_items))
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("VelocityRL API Server running on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_get_items() -> Json<Value> {
    // In a real scenario, this would authenticate and fetch from Psynet.
    // For now, we return a sample response or attempt a fetch if keys are provided.
    
    // Attempt to fetch from Psynet (requires valid session/token logic)
    // For the hosted API, we'd typically have a background worker updating a cache.
    
    let sample = json!({
        "Items": [
            {
                "ID": 1,
                "Product": "Standard Boost",
                "AssetPackage": "Standard_Boost",
                "AssetPath": "Boost_Standard.Standard",
                "Slot": "Boost"
            }
        ]
    });

    Json(sample)
}
