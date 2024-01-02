#[macro_use] extern crate rocket;

use rocket_dyn_templates::Template;

mod routes;
mod settings;
mod repo;
mod sessions;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes::routes())
        .attach(Template::fairing())
}
