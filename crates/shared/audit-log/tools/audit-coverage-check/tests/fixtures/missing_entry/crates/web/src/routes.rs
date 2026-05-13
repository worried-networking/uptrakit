async fn test_create_item() {}

pub fn router() {
    let _r = axum::Router::<()>::new().route("/items", axum::routing::post(test_create_item));
}
