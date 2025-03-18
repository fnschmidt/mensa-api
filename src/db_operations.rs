use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;

use crate::{stuwe_request_funcs::build_date_string, types::MealGroup};

const DB_FILENAME: &str = "meals.sqlite";

pub fn check_or_create_db_tables() -> rusqlite::Result<()> {
    let conn = Connection::open(DB_FILENAME)?;

    // table of all canteens
    conn.prepare(
        "create table if not exists mensen (
            mensa_id integer primary key,
            mensa_name text not null unique
        )",
    )?
    .execute([])?;

    // table of all meals
    conn.prepare(
        "create table if not exists meals (
            mensa_id integer,
            date text,
            json_text text,
            foreign key (mensa_id) references mensen(mensa_id)
        )",
    )?
    .execute([])?;

    Ok(())
}

pub fn add_canteen_id_db(id: u32, name: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(DB_FILENAME)?;
    let mut stmt = conn.prepare_cached(
        "replace into mensen (mensa_id, mensa_name)
            values (?1, ?2)",
    )?;
    stmt.execute(params![id, name])?;

    Ok(())
}

pub async fn save_meal_to_db(date: &str, canteen_id: u32, json_text: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(DB_FILENAME)?;
    conn.execute(
        "delete from meals where mensa_id = ?1 and date = ?2",
        params![canteen_id, date],
    )?;

    let mut stmt = conn.prepare_cached(
        "insert into meals (mensa_id, date, json_text)
            values (?1, ?2, ?3)",
    )?;

    stmt.execute(params![canteen_id, date, json_text])?;

    Ok(())
}

pub async fn get_canteens_from_db() -> Result<BTreeMap<u32, String>> {
    let conn = Connection::open(DB_FILENAME)?;
    let mut stmt = conn.prepare("select mensa_id, mensa_name from mensen")?;

    let canteen_iter = stmt.query_map([], |row| {
        Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut canteens = BTreeMap::new();
    for canteen in canteen_iter {
        let (canteen_id, canteen_name) = canteen?;
        canteens.insert(canteen_id, canteen_name);
    }

    Ok(canteens)
}

pub fn list_available_days_db(canteen_id: u32) -> rusqlite::Result<Vec<String>> {
    let conn = Connection::open(DB_FILENAME)?;
    let mut stmt = conn.prepare_cached("select date from meals where mensa_id = ?1")?;
    let mut rows = stmt.query(params![canteen_id])?;

    let mut dates = vec![];
    while let Some(row) = rows.next()? {
        dates.push(row.get(0)?);
    }

    Ok(dates)
}

pub async fn get_meals_from_db(
    canteen_id: u32,
    requested_date: NaiveDate,
) -> Result<Vec<MealGroup>> {
    let date_str = build_date_string(requested_date);
    let json_text = get_jsonmeals_from_db(&date_str, canteen_id).await?;
    if let Some(json_text) = json_text {
        json_to_meal(&json_text).await
    } else {
        Ok(vec![])
    }
}

async fn json_to_meal(json_text: &str) -> Result<Vec<MealGroup>> {
    Ok(serde_json::from_str(json_text)?)
}

pub async fn get_jsonmeals_from_db(
    date: &str,
    canteen_id: u32,
) -> rusqlite::Result<Option<String>> {
    let conn = Connection::open(DB_FILENAME)?;
    let mut stmt =
        conn.prepare_cached("select json_text from meals where (mensa_id, date) = (?1, ?2)")?;
    let mut rows = stmt.query(params![canteen_id, date])?;

    Ok(rows.next().unwrap().map(|row| row.get(0).unwrap()))
}
