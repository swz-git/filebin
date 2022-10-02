use actix_web::{get, services, web, App, Handler, Responder};
use sled::Db;

#[get("/")]
async fn root(db: web::Data<Db>) -> impl Responder {
    db.get("a").expect("shit");
    format!("API Is live")
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/api").service(root));
}
