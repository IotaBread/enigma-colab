#[macro_use] extern crate rocket;

use rocket::fairing::AdHoc;
use rocket::State;
use rocket::tokio::sync::Mutex;
use rocket_dyn_templates::Template;

use crate::sessions::Session;

mod routes;
mod settings;
mod repo;
mod sessions;

type SessionList = Mutex<Vec<Session>>;
type SessionsState<'r> = &'r State<SessionList>;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes::routes())
        .attach(Template::fairing())
        .attach(AdHoc::try_on_ignite("Sessions", |rocket| async {
            let sessions = match sessions::load_sessions() {
                Ok(s) => s,
                Err(e) => panic!("Failed to load the sessions: {e}"),
            };

            Ok(rocket.manage(SessionList::new(sessions)))
        }))
}
