use std::convert::Infallible;
use std::env;

use crypto::digest::Digest;
use crypto::sha3::Sha3;
use rocket::{Request, Route};
use rocket::form::Form;
use rocket::http::{CookieJar, Status};
use rocket::outcome::IntoOutcome;
use rocket::outcome::Outcome::Forward;
use rocket::request::{FlashMessage, FromRequest, Outcome};
use rocket::response::{Flash, Redirect};
use rocket::serde::Deserialize;
use rocket_dyn_templates::{context, Template};

use crate::repo;
use crate::settings;
use crate::settings::{RepoSettings, Settings};

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

#[derive(FromForm, Deserialize)]
struct SettingsData {
    jar_file: String,
    mappings_file: String,
    auto_save_interval: u16,
    pull_cmd: String,
    pre_session_cmd: String,
    post_session_cmd: String,
    enigma_args: String,
    classpath: String,
}

impl SettingsData {
    fn write(self, settings: &mut Settings) {
        settings.jar_file = self.jar_file;
        settings.mappings_file = self.mappings_file;
        settings.auto_save_interval = self.auto_save_interval;
        settings.pull_cmd = self.pull_cmd;
        settings.pre_session_cmd = self.pre_session_cmd;
        settings.post_session_cmd = self.post_session_cmd;
        settings.enigma_args = self.enigma_args;
        settings.classpath = self.classpath;
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
    Template::render("login", context! {
        logged_in: false
    })
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
async fn settings_page(admin_user: AdminUser, flash: Option<FlashMessage<'_>>) -> Template {
    let (settings, err) = match settings::read_settings().await {
        Ok(s) => (s, None),
        Err(e) => (Settings::default(), Some(e.to_string()))
    };

    let cloned = repo::is_cloned();
    let branches = if cloned {
        Some(match repo::list_branches().await {
            Ok(b) => b,
            Err(_) => Vec::new(),
        })
    } else {
        None
    };

    Template::render("settings", context! {
        logged_in: true,
        admin: true,
        settings: settings,
        cloned: cloned,
        error: err,
        msg: flash,
        branches: branches,
    })
}

async fn edit_settings<T: FnOnce(&mut Settings)>(redirect: Redirect, t: T) -> Flash<Redirect> {
    let mut settings = match settings::read_settings().await {
        Ok(s) => s,
        Err(e) => {
            println!("{}", e);
            return Flash::error(redirect, e.to_string())
        }
    };

    t(&mut settings);

    match settings::write_settings(&settings).await {
        Ok(_) => Flash::success(redirect, "Settings updated"),
        Err(e) => {
            println!("{}", e);
            Flash::error(redirect, e.to_string())
        }
    }
}

#[post("/settings", data = "<settings_data>")]
async fn post_settings(admin_user: AdminUser, settings_data: Form<SettingsData>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(index));

    edit_settings(redirect, |settings| settings_data.into_inner().write(settings)).await
}

#[post("/settings/repo", data = "<repo_settings>")]
async fn post_repo_settings(admin_user: AdminUser, repo_settings: Form<RepoSettings>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));

    edit_settings(redirect, |settings| settings.repo = repo_settings.into_inner()).await
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
async fn clone_repo(admin: AdminUser) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));
    if repo::is_cloned() {
        return Flash::error(redirect, "A repository already exists, can't clone");
    }

    match repo::clone().await {
        Ok((branch, rev)) =>
            Flash::success(redirect, format!("Cloned repo, with branch '{branch}' at {rev}")),
        Err(e) => Flash::error(redirect, e.to_string())
    }
}

#[post("/fetch")]
fn fetch(admin: AdminUser) {
}

#[post("/pull")]
fn pull(admin: AdminUser) {
}

#[get("/sessions/new")]
fn new_session_page(admin_user: AdminUser) -> Template {
    Template::render("new_session", context! {
        logged_in: true,
        admin: true
    })
}

#[post("/sessions/new")]
fn new_session_form(admin_user: AdminUser) -> Flash<Redirect> {
    Flash::success(Redirect::to(uri!(index)), "New session created placeholder")
}

pub fn routes() -> Vec<Route> {
    routes![index,
        login, login_page, login_form, logout,
        settings_page, post_settings, post_repo_settings, settings_unauthorized, settings_redirect, clone_repo,
        new_session_page, new_session_form]
}
