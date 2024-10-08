use std::convert::Infallible;
use std::env;

use rocket::{Request, Route};
use rocket::form::Form;
use rocket::fs::NamedFile;
use rocket::http::{CookieJar, Status};
use rocket::outcome::IntoOutcome;
use rocket::outcome::Outcome::Forward;
use rocket::request::{FlashMessage, FromRequest, Outcome};
use rocket::response::{Flash, Redirect};
use rocket::serde::Deserialize;
use rocket_dyn_templates::{context, Template};
use uuid::Uuid;

use crate::{repo, SessionsState, util};
use crate::sessions::Session;
use crate::settings;
use crate::settings::{RepoSettings, Settings};

#[derive(FromForm)]
struct Login<'r> {
    user: &'r str,
    password: &'r str
}

#[derive(FromForm)]
struct NewSession<'r> {
    password: &'r str,
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
        let id = env::var("ADMIN_SESSION_ID");

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

#[get("/login")]
fn login(_user: User) -> Redirect {
    Redirect::to(uri!(index))
}

#[get("/login", rank = 2)]
fn login_page(flash: Option<FlashMessage<'_>>) -> Template {
    Template::render("login", context! {
        logged_in: false,
        msg: flash
    })
}

#[post("/login", data = "<login>")]
fn login_form(cookies: &CookieJar<'_>, login: Form<Login<'_>>) -> Flash<Redirect> {
    // TODO: Users, registration, database
    let user = env::var("USER");
    let password = env::var("PASSWORD_HASH");
    if user.is_ok() && password.is_ok() {
        if login.user == user.unwrap() && util::sha3_256(&login.password) == password.unwrap() {
            let id = env::var("ADMIN_SESSION_ID");
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
async fn settings_page(_admin_user: AdminUser, flash: Option<FlashMessage<'_>>) -> Template {
    let (settings, err) = match settings::read_settings().await {
        Ok(s) => (s, None),
        Err(e) => (Settings::default(), Some(format!("Failed to read settings: {e}")))
    };

    let cloned = repo::is_cloned();
    let branches = if cloned {
        Some(match repo::list_local_branches().await {
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

async fn update_settings<T: FnOnce(&mut Settings)>(updater: T) -> Option<String> {
    let mut settings = match settings::read_settings().await {
        Ok(s) => s,
        Err(e) => {
            println!("{}", e);
            return Some(format!("Failed to read settings: {e}"))
        }
    };

    updater(&mut settings);

    match settings::write_settings(&settings).await {
        Ok(_) => None,
        Err(e) => {
            println!("{}", e);
            Some(format!("Failed to write settings: {e}"))
        }
    }
}

#[post("/settings", data = "<settings_data>")]
async fn post_settings(_admin_user: AdminUser, settings_data: Form<SettingsData>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(index));

    match update_settings(|settings| settings_data.into_inner().write(settings)).await {
        Some(msg) => Flash::error(redirect, msg),
        None => Flash::success(redirect, "Settings updated")
    }
}

#[post("/settings/repo", data = "<repo_settings>")]
async fn post_repo_settings(_admin_user: AdminUser, repo_settings: Form<RepoSettings>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));

    match update_settings(|settings| settings.repo = repo_settings.into_inner()).await {
        Some(msg) => Flash::error(redirect, msg),
        None => Flash::success(redirect, "Settings updated")
    }
}

#[get("/settings", rank = 2)]
fn settings_unauthorized(_user: User) -> Status {
    Status::Unauthorized
}

#[get("/settings", rank = 3)]
fn settings_redirect() -> Redirect {
    Redirect::to(uri!(login))
}

#[get("/")]
async fn index(user: Option<User>, flash: Option<FlashMessage<'_>>, sessions: SessionsState<'_>) -> Template {
    let mut sessions = sessions.lock().await;
    let mut running = vec![];
    let mut recent = vec![];

    let mut iter = sessions.iter_mut();
    while let Some(session) = iter.next() {
        if session.check_is_running().expect("Failed to check the session status") {
            running.push(session);
        } else {
            recent.push(session);
        }
    }

    Template::render("index", context! {
        logged_in: user.is_some(),
        admin: user.filter(|v| {v.0 == env::var("ADMIN_SESSION_ID").unwrap_or_default()}).is_some(),
        msg: flash,
        cloned: repo::is_cloned(),
        sessions: context! {
            running,
            recent
        }
    })
}

#[post("/clone")]
async fn clone_repo(_admin: AdminUser) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));
    if repo::is_cloned() {
        return Flash::error(redirect, "A repository already exists, can't clone");
    }

    // TODO: Send "cloning..." response, update once done?
    match repo::clone().await {
        Ok((branch, rev)) =>
            Flash::success(redirect, format!("Cloned repo, with branch '{branch}' at {rev}")),
        Err(e) => Flash::error(redirect, format!("Failed to clone repo: {e}"))
    }
}

#[post("/fetch")]
async fn fetch(_admin_user: AdminUser) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));
    match repo::fetch() {
        Ok(_) => Flash::success(redirect, "Fetched remote"),
        Err(e) => Flash::error(redirect, format!("Failed to fetch repo: {e}"))
    }
}

#[post("/pull")]
async fn pull(_admin_user: AdminUser) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));

    match repo::pull().await {
        Ok(res) => { match res {
            Ok(rev) => Flash::success(redirect, format!("Pulled remote: HEAD is now at {rev}")),
            Err(msg) => Flash::success(redirect, format!("Not updated: {msg}"))
        } },
        Err(e) => Flash::error(redirect, format!("Failed to pull from repo: {e}"))
    }
}

#[post("/checkout", data = "<repo_settings>")]
async fn checkout(_admin_user: AdminUser, repo_settings: Form<RepoSettings>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(settings_page));

    let branch = repo_settings.branch.clone();
    if let Some(msg) = update_settings(|settings| settings.repo = repo_settings.into_inner()).await {
        return Flash::error(redirect, msg);
    }

    match repo::checkout().await {
        Ok(rev) => Flash::success(redirect, format!("Checked out {branch}: HEAD is now at {rev}")),
        Err(e) => Flash::error(redirect, format!("Failed to checkout {branch}: {e}"))
    }
}

#[get("/sessions/new")]
fn new_session_page(_admin_user: AdminUser) -> Template {
    Template::render("new_session", context! {
        logged_in: true,
        admin: true
    })
}

#[post("/sessions/new", data = "<data>")]
async fn new_session_form(_admin_user: AdminUser, sessions: SessionsState<'_>, data: Form<NewSession<'_>>) -> Flash<Redirect> {
    let error_redirect = Redirect::to(uri!(index));

    if !repo::is_cloned() {
        return Flash::error(error_redirect, "Repo not cloned");
    }

    let mut sessions = sessions.lock().await;
    let session = match Session::new(Some(data.password.to_string())).await {
        Ok(s) => s,
        Err(e) => { return Flash::error(error_redirect, format!("Failed to start session: {e}")); },
    };
    let redirect = Redirect::to(uri!(session_page(session.id)));
    sessions.push(session);

    Flash::success(redirect, "New session started")
}

#[get("/sessions/<id>")]
async fn session_page(id: Uuid, user: Option<User>, flash: Option<FlashMessage<'_>>, sessions: SessionsState<'_>) -> Option<Template> {
    let sessions = sessions.lock().await;
    let session = sessions.iter().find(|s| s.id == id)?;

    Some(Template::render("session", context! {
        logged_in: user.is_some(),
        admin: user.filter(|v| {v.0 == env::var("ADMIN_SESSION_ID").unwrap_or_default()}).is_some(),
        msg: flash,
        session: session
    }))
}

#[get("/sessions/<id>/patch")]
async fn session_patch(id: Uuid, sessions: SessionsState<'_>) -> Option<NamedFile> {
    let sessions = sessions.lock().await;
    let session = sessions.iter().find(|s| s.id == id)?;

    let file_path = session.get_patch_file();
    let file_path = file_path.as_path();
    if file_path.exists() {
        if let Ok(file) = NamedFile::open(file_path).await {
            Some(file)
        } else {
            None
        }
    } else {
        None
    }
}

#[get("/sessions/<id>/log")]
async fn session_log(id: Uuid, _admin_user: AdminUser) -> &'static str {
    // TODO
    "Session log goes here"
}

#[post("/sessions/<id>/finish")]
async fn finish_session(id: Uuid, _admin_user: AdminUser, sessions: SessionsState<'_>) -> Flash<Redirect> {
    let redirect = Redirect::to(uri!(session_page(id)));
    let mut sessions = sessions.lock().await;

    if let Some(session) = sessions.iter_mut().find(|s| s.id == id) {
        match session.finish().await {
            Ok(_) => Flash::success(redirect, "Session finished"),
            Err(e) => Flash::error(redirect, format!("Failed to end session: {e}"))
        }
    } else {
        Flash::error(Redirect::to(uri!(index)), "Session not found")
    }
}

pub fn routes() -> Vec<Route> {
    routes![index,
        login, login_page, login_form, logout,
        settings_page, post_settings, post_repo_settings, settings_unauthorized, settings_redirect,
        clone_repo, fetch, pull, checkout,
        new_session_page, new_session_form, session_page, session_patch, session_log, finish_session]
}
