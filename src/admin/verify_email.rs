use crate::{db, schema::emails, util::errors::AppResult};
use diesel::prelude::*;
use diesel::update;

use clap::Clap;

#[derive(Clap, Debug)]
#[clap(
    name = "verify-email",
    about = "Verify email by email address",
    long_about = "Set status of an email address to verified. \
        Useful in developer environments"
)]

pub struct Opts {
    email: String,
}

pub fn run(opts: Opts) -> AppResult<()> {
    let conn = db::connect_now()?;
    let res = update(emails::table.filter(emails::email.eq(opts.email.clone())))
        .set(emails::verified.eq(true))
        .execute(&conn)?;
    if res >= 1 {
        println!("Email {} is now verified", opts.email);
    } else {
        println!("Email {} was not found", opts.email);
    }  
    Ok(())
}
