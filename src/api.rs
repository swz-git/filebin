use rocket::{Route, State};
use sled::Db;

#[get("/")]
fn index(db: &State<Db>) -> &'static str {
    log::debug!("{:?}", db.get("a").expect("shit"));
    "API Is live"
}

pub fn get_routes() -> Vec<Route> {
    routes![index]
}
