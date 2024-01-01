use std::convert::Infallible;
use std::env;
use crypto::digest::Digest;
use crypto::sha3::Sha3;
use rocket::form::Form;
use rocket::http::{CookieJar, Status};
use rocket::request::{FlashMessage, FromRequest, Outcome};
use rocket::response::{Flash, Redirect};
use rocket::{Request, Route};
use rocket::outcome::IntoOutcome;
use rocket::outcome::Outcome::Forward;
use rocket_dyn_templates::{context, Template};
use crate::settings;
use crate::settings::Settings;

#[derive(FromForm)]
struct Login<'r> {
    user: &'r str,
    password: &'r str
}

#[derive(Debug)]
struct User(String);

#[derive(Debug)]
struct AdminUser(String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for User {
    type Error = Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        request.cookies()
            .get_private("session")
            .and_then(|cookie| cookie.value().parse().ok())
            .map(User)
            .or_forward(Status::Unauthorized)
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminUser {
    type Error = Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let id = env::var("SESSION_ID");

        if id.is_ok() {
            request.cookies()
                .get_private("session")
                .and_then(|cookie| cookie.value().parse().ok())
                .filter(|v| *v == id.unwrap())
                .map(AdminUser)
                .or_forward(Status::Unauthorized)
        } else {
            Forward(Status::Unauthorized)
        }
    }
}

fn hash_password(password: &str) -> String {
    let mut hasher = Sha3::sha3_256();
    hasher.input_str(password);
    hasher.result_str()
}

#[get("/login")]
fn login(user: User) -> Redirect {
    Redirect::to(uri!(index))
}

#[get("/login", rank = 2)]
fn login_page() -> Template {
    Template::render("login", context! {})
}

#[post("/login", data = "<login>")]
fn login_form(cookies: &CookieJar<'_>, login: Form<Login<'_>>) -> Flash<Redirect> {
    // TODO: Users, registration, database
    let user = env::var("USER");
    let password = env::var("PASSWORD_HASH");
    if user.is_ok() && password.is_ok() {
        if login.user == user.unwrap() && hash_password(&login.password) == password.unwrap() {
            let id = env::var("SESSION_ID");
            if id.is_ok() {
                cookies.add_private(("session", id.unwrap()));
            }

            return Flash::success(Redirect::to(uri!(index)), "Logged in");
        }
    }

    Flash::error(Redirect::to(uri!(login_page)), "Invalid user/password")
}

#[get("/logout")]
fn logout(cookies: &CookieJar<'_>) -> Flash<Redirect> {
    cookies.remove_private("session");
    Flash::success(Redirect::to(uri!(index)), "Logged out")
}

#[get("/settings")]
async fn settings_page(admin_user: AdminUser) -> Template {
    let (settings, err) = match settings::read_settings().await {
        Ok(s) => (s, None),
        Err(e) => (Settings::default(), Some(e))
    };

    Template::render("settings", context! {
        settings: settings,
        error: err.map(|t| {t.to_string()})
    })
}

#[post("/settings", data = "<settings>")]
async fn post_settings(admin_user: AdminUser, settings: Form<Settings>) -> Flash<Redirect> {
    match settings::write_settings(&settings.into_inner()).await {
        Ok(_) => Flash::success(Redirect::to(uri!(index)), "Settings updated"),
        Err(e) => {
            println!("{}", e);
            Flash::error(Redirect::to(uri!(index)), e.to_string())
        }
    }
}

#[get("/settings", rank = 2)]
fn settings_unauthorized(user: User) -> Status {
    Status::Unauthorized
}

#[get("/settings", rank = 3)]
fn settings_redirect() -> Redirect {
    Redirect::to(uri!(login))
}

#[get("/")]
fn index(user: Option<User>, flash: Option<FlashMessage<'_>>) -> Template {
    Template::render("index", context! {
        logged_in: user.is_some(),
        admin: user.filter(|v| {v.0 == env::var("SESSION_ID").unwrap_or_default()}).is_some(),
        msg: flash
    })
}

#[post("/clone")]
fn clone(admin: AdminUser) {
}

#[post("/fetch")]
fn fetch(admin: AdminUser) {
}

#[post("/pull")]
fn pull(admin: AdminUser) {
}

pub fn routes() -> Vec<Route> {
    routes![index, login, login_page, login_form, logout, settings_page, post_settings, settings_unauthorized, settings_redirect]
}
