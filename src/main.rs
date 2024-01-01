#[macro_use] extern crate rocket;

use rocket_dyn_templates::Template;
use serde::{Deserialize, Serialize};

mod routes;
mod settings;
mod repo;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes::routes())
        .attach(Template::fairing())
}
