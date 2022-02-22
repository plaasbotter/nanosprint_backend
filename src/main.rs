use actix_web::{get, post, put, web, App, HttpResponse, HttpServer, Responder};
use actix_cors::Cors;
use chrono::prelude::*;
use chrono::{DateTime, NaiveDateTime, Utc};
use dotenv::dotenv;
use postgres::{Client, NoTls};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::time::UNIX_EPOCH;

//GET NANODYN SPRINT STUFF
#[get("/nsprinter")]
async fn get_sprint_data() -> impl Responder {
    //LOAD BASIC VARIABLES
    let mut max_db_header: i32 = 0;
    let mut row_sprint_map: HashMap<i32, i32> = HashMap::new();
    let mut db_activites: Vec<DBActivities> = Vec::new();

    println!("Entrance from get_sprint_data");
    println!("Connecting to database");

    let mut client = Client::connect(
        env::var("DATABASE_URL")
            .expect("Please set the DB URL in env.")
            .as_str(),
        NoTls,
    )
    .expect("Client could not connect");

    //GET SPRINTS
    let db_sprints: Vec<DBSprints> = get_sprints(&mut client);

    //GET HEADERS
    let db_headers: Vec<DBHeaders> = get_headers(&mut client);

    //GET ACTIVITIES
    for sprint in &db_sprints {
        let query = format!(
            "SELECT * FROM activities WHERE \"sID\" = {} AND \"hID\" > 0",
            sprint.s_id
        );
        for row in client.query(query.as_str(), &[]).unwrap() {
            let temp = DBActivities {
                a_id: row.get(0),
                s_id: row.get(1),
                h_id: row.get(2),
                value: row.get(3),
            };
            db_activites.push(temp);
        }
    }

    // GET MAX VALUE
    let query = "SELECT MAX(ABS(\"hID\")) FROM \"headers\"";
    for row in client.query(query, &[]).unwrap() {
        max_db_header = row.get(0);
    }

    //END OF DATABASESEGMENT
    client.close().unwrap();

    //GENERATE DATUM
    let mut datum = Data {
        headerMax: max_db_header,
        activityHeaders: Vec::new(),
        headerCount: db_headers.len(),
        sprintNumber: db_sprints[0].number,
        rows: Vec::new(),
    };

    //GENERATE HEADER
    for header in db_headers {
        datum.activityHeaders.push(Header {
            headerID: header.h_id,
            headerName: header.name,
        });
    }

    //LOAD WEEK NUMBER
    let mut counter = 1;
    for sprint in db_sprints {
        datum.rows.push(Row {
            rowID: counter,
            date: sprint.date.format("%e %B").to_string(),
            activities: Vec::new(),
        });
        row_sprint_map.insert(sprint.s_id, counter);
        counter = counter + 1;
    }

    //GENERATE FULL DATA
    for activity in db_activites {
        for item in &mut datum.rows {
            if item.rowID == row_sprint_map[&activity.s_id] {
                item.activities.push(Activity {
                    headerID: activity.h_id,
                    value: activity.value.to_string(),
                    activityID: activity.a_id,
                });
            }
        }
    }

    return web::Json(datum);
}

#[put("/nsprinter")]
async fn set_sprint_data(req_body: String) -> impl Responder {
    match serde_json::from_str::<Data>(req_body.as_str()) {
        Ok(datum) => {
            println!("Entrance from set_sprint_data");
            println!("Successfully received data, inserting into DB");
            println!("Connecting to database");

            //DECLARATION STAION
            let mut row_sprint_map: HashMap<i32, i32> = HashMap::new();

            //CLIENT CONNENCTION
            let mut client = Client::connect(
                env::var("DATABASE_URL")
                    .expect("Please set the DB URL in env.")
                    .as_str(),
                NoTls,
            )
            .expect("Client could not connect");

            //GET SPRINTS
            let db_sprints: Vec<DBSprints> = get_sprints(&mut client);

            //CREATE SPRINT HASH MAP
            let mut counter = 1;
            for sprint in db_sprints {
                row_sprint_map.insert(counter, sprint.s_id);
                counter = counter + 1;
            }

            //UPDATE HEADERS
            for header in datum.activityHeaders {
                if header.headerID <= 0 {
                    let query = format!(
                        "UPDATE \"headers\" SET \"hID\" = {} WHERE \"hID\" = {}",
                        header.headerID,
                        (0 - header.headerID)
                    );
                    client
                        .execute(query.as_str(), &[])
                        .expect("Could not toggle headers");
                    let query = format!(
                        "UPDATE \"activities\" SET \"hID\" = {} WHERE \"hID\" = {}",
                        header.headerID,
                        (0 - header.headerID)
                    );
                    client
                        .execute(query.as_str(), &[])
                        .expect("Could not toggle activities");
                } else {
                    let query = format!(
                            "INSERT INTO \"headers\" (\"hID\",\"name\") VALUES ({},\'{}\') ON CONFLICT (\"hID\") DO UPDATE SET \"name\" = EXCLUDED.\"name\"",
                            header.headerID, header.headerName
                        );
                    client
                        .execute(query.as_str(), &[])
                        .expect("Could not upsert headers");
                }
            }

            //LOOP THROUGH ACTIVITIES
            for row in datum.rows {
                for activity in row.activities {
                    if activity.activityID == 0 {
                        let query = format!(
                                "INSERT INTO \"activities\" (\"sID\",\"hID\",\"value\") VALUES ({},{},\'{}\')",
                                row_sprint_map[&row.rowID], activity.headerID, activity.value
                            );
                        client
                            .execute(query.as_str(), &[])
                            .expect("Could not insert new activity");
                    } else {
                    let query = format!("INSERT INTO \"activities\" (\"aID\",\"sID\",\"hID\",\"value\") VALUES ({},{},{},\'{}\') ON CONFLICT (\"aID\") DO UPDATE SET \"sID\" = EXCLUDED.\"sID\", \"hID\" = EXCLUDED.\"hID\", \"value\" = EXCLUDED.\"value\"",
                            activity.activityID, row_sprint_map[&row.rowID],activity.headerID,activity.value
                        );
                    client
                        .execute(query.as_str(), &[])
                        .expect("Could not upsert activity");
                    }

                }
            }
            //CLOSE CLIENT
            client.close().unwrap();
        }
        Err(e) => {
            println!("Error converting: {:#?}", e);
            return HttpResponse::BadRequest();
        }
    };
    return HttpResponse::Ok();
}

#[post("/nsprinter")]
async fn new_sprint_data(req_body: String) -> impl Responder {
    match serde_json::from_str::<NewSprint>(req_body.as_str()) {
        Ok(sprint_details) => {
            //GET AND REPORT DETAILS
            println!("Entrance from new_sprint_data");
            let tempdate =
                NaiveDate::parse_from_str(sprint_details.date.as_str(), "%Y-%m-%d").unwrap();
            let num_days_from_ce = tempdate.num_days_from_ce();
            let mut client = Client::connect(
                env::var("DATABASE_URL")
                    .expect("Please set the DB URL in env.")
                    .as_str(),
                NoTls,
            )
            .expect("Client could not connect");

            //INSERT NEW SPRINT
            for i in 0..14 {
                let newdate = NaiveDate::from_num_days_from_ce(num_days_from_ce + i);
                let query = format!(
                    "INSERT INTO \"sprints\" (\"date\",\"number\") VALUES (\'{}\',{})",
                    newdate, sprint_details.sprintNumber
                );

                client
                    .execute(query.as_str(), &[])
                    .expect("Could not insert new activity");
            }

            //DECLARE VARIABLES
            let db_sprints: Vec<DBSprints> = get_sprints(&mut client);
            let db_headers: Vec<DBHeaders> = get_headers(&mut client);

            for db_sprint in &db_sprints {
                for db_header in &db_headers {
                    let query = format!( 
                        "INSERT INTO \"activities\" (\"sID\",\"hID\",\"value\") VALUES ({},{},\'\')",
                        db_sprint.s_id, db_header.h_id
                    );
                    client
                        .execute(query.as_str(), &[])
                        .expect("Could not insert new activity");
                }
            }

            //CLOSE CLIENT
            client.close().unwrap();
        }
        Err(e) => {
            println!("Error converting: {:#?}", e);
            return HttpResponse::BadRequest();
        }
    };
    return HttpResponse::Ok();
}

//GET SPRINTS
fn get_sprints(client: &mut Client) -> Vec<DBSprints> {
    let mut db_sprints: Vec<DBSprints> = Vec::new();
    let query = "SELECT * FROM sprints ORDER BY date DESC LIMIT 14";
    for row in client.query(query, &[]).unwrap() {
        let timestamp: std::time::SystemTime = row.get(1);
        let timestamp = timestamp.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let temp_date = timestamp as i64;
        let temp_date = NaiveDateTime::from_timestamp(temp_date, 0);
        let datetime: DateTime<Utc> = DateTime::from_utc(temp_date, Utc);
        let temp = DBSprints {
            s_id: row.get(0),
            date: datetime,
            number: row.get(2),
        };
        db_sprints.push(temp);
    }
    db_sprints.reverse();
    return db_sprints;
}

//GET ALL HEADERS
fn get_headers(client: &mut Client) -> Vec<DBHeaders> {
    let mut db_headers: Vec<DBHeaders> = Vec::new();
    let query = "SELECT * FROM headers WHERE \"hID\" > 0";
    for row in client.query(query, &[]).unwrap() {
        let temp = DBHeaders {
            h_id: row.get(0),
            name: row.get(1),
        };
        db_headers.push(temp);
    }
    return db_headers;
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    let mut host = env::var("HOST").expect("Please set host in .env");
    {
        let environment = env::consts::OS;
        let mut command = "ifconfig";
        if environment == "windows" {
            command = "ipconfig";
        }
        let output = Command::new(command).output().expect("ERROR ERROR");
        let kickass = String::from_utf8(output.stdout).unwrap();
        let re = Regex::new(r#"(([0-9]*\.){3}[0-9]*)"#).unwrap();
        for cap in re.captures_iter(&kickass) {
            let temp_ip_str = cap.get(0).map_or("", |m| m.as_str());
            let temp_split: Vec<&str> = temp_ip_str.split(".").collect();
            let val4 = match temp_split.get(3) {
                Some(it) => it,
                None => "1",
            };
            let val1 = match temp_split.get(0) {
                Some(it) => it,
                None => "1",
            };
            if val4 != "255" && val4 != "1" && val4 != "0" && val1 != "255" {
                host = temp_ip_str.into();
            }
        }
    }
    let port = env::var("PORT").expect("Please set port in .env");
    println!("HOSTING ON: {}:{}", host, port);
    HttpServer::new(|| {
      Cors::default().supports_credentials();
        App::new()
            .service(get_sprint_data)
            .service(set_sprint_data)
            .service(new_sprint_data)
        //.service(echo)
    })
    .bind(format!("{}:{}", host, port))?
    .run()
    .await
}

#[derive(Debug)]
struct DBActivities {
    a_id: i32,
    s_id: i32,
    h_id: i32,
    value: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
struct NewSprint {
    date: String,
    sprintNumber: i32,
}

#[derive(Debug)]
struct DBHeaders {
    h_id: i32,
    name: String,
}

#[derive(Debug)]
struct DBSprints {
    s_id: i32,
    date: DateTime<Utc>,
    number: i32,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
struct Data {
    activityHeaders: Vec<Header>,
    headerCount: usize,
    rows: Vec<Row>,
    sprintNumber: i32,
    headerMax: i32,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
struct Activity {
    headerID: i32,
    value: String,
    activityID: i32,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
struct Row {
    rowID: i32,
    date: String,
    activities: Vec<Activity>,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_snake_case)]
struct Header {
    headerID: i32,
    headerName: String,
}
