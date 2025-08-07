use actix_cors::Cors;
use actix_files::Files;
use actix_web::{
    get, middleware::Logger, web, App, HttpMessage, HttpRequest, HttpResponse, HttpServer,
};
use chrono::NaiveDate;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    cell::Cell,
    fs::File,      // برای خواندن فایل
    io::BufReader, // برای خواندن بهینه فایل
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio_postgres::{Error, NoTls, Row};

// ساختار جدید برای نگهداری تنظیمات خوانده شده از config.json
#[derive(Deserialize)]
struct Config {
    port: u16,
}

// This struct represents state
struct AppState {
    _app_name: String,
    _count: Cell<usize>,
    global_count: Arc<AtomicUsize>,
}

#[derive(Deserialize)]
struct KInfo {
    username: String,
    name: String,
    id: u32,
}

#[get("/")]
async fn kindex(
    req: HttpRequest,
    info: web::Query<KInfo>,
    data: web::Data<AppState>,
) -> HttpResponse {
    let previous = data.global_count.fetch_add(3, Ordering::SeqCst);
    let current = data.global_count.load(Ordering::SeqCst);

    let visit_count = if let Ok(cookies) = req.cookies() {
        cookies
            .iter()
            .find(|c| c.name() == "visit_count")
            .and_then(|c| c.value().parse::<i32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    let new_visit_count = visit_count + 1;

    // Enhanced connection information
    let connection_info = req.connection_info();
    let (client_ip, client_port) = connection_info
        .peer_addr()
        .unwrap_or("unknown")
        .split_once(':')
        .unwrap_or(("unknown", "unknown"));

    let mut request_info = vec![
        format!("=== Visit Counter ==="),
        format!("This is visit number: {}", new_visit_count),
        format!("\n=== Detailed Connection Info ==="),
        format!("Client IP: {}", client_ip),
        format!("Client Port: {}", client_port),
        format!("Server Address: {}", connection_info.host()),
        format!("Connection Scheme: {}", connection_info.scheme()),
        format!(
            "Real IP: {}",
            connection_info.realip_remote_addr().unwrap_or("unknown")
        ),
        format!("\n=== Request Details ==="),
        format!("Method: {}", req.method()),
        format!("Version: {:?}", req.version()),
        format!("URI: {}", req.uri()),
        format!("Path: {}", req.path()),
        format!("Query String: {}", req.query_string()),
        format!("\n=== Security Info ==="),
        format!(
            "Secure Connection: {}",
            req.connection_info().scheme() == "https"
        ),
        format!(
            "Forwarded Proto: {:?}",
            req.headers().get("x-forwarded-proto")
        ),
        format!("Forwarded For: {:?}", req.headers().get("x-forwarded-for")),
        format!("\n=== Network Details ==="),
        format!("Remote Address: {:?}", req.peer_addr()),
    ];

    // Add all headers with detailed information
    request_info.push(format!("\n=== Headers ==="));
    for (header_name, header_value) in req.headers() {
        request_info.push(format!(
            "{}: {:?}",
            header_name,
            header_value.to_str().unwrap_or("Unable to read value")
        ));
    }

    // Add extended request information
    request_info.extend(vec![
        format!("\n=== Technical Details ==="),
        format!("Content Type: {:?}", req.content_type()),
        format!("Content Length: {:?}", req.headers().get("content-length")),
        format!("Encoding: {:?}", req.encoding()),
        format!("Match Info: {:?}", req.match_info()),
        format!(
            "App State Counter - Previous: {}, Current: {}",
            previous, current
        ),
        format!("\n=== Query Parameters ==="),
        format!("Username: {}", info.username),
        format!("Name: {}", info.name),
        format!("ID: {}", info.id),
    ]);

    // Add cookie information
    if let Ok(cookies) = req.cookies() {
        request_info.push(format!("\n=== Cookies ==="));
        request_info.push(format!("Cookie Count: {}", cookies.len()));
        for cookie in cookies.iter() {
            request_info.push(format!(
                "Cookie {}: {} (Secure: {}, HttpOnly: {})",
                cookie.name(),
                cookie.value(),
                cookie.secure().unwrap_or(false),
                cookie.http_only().unwrap_or(false)
            ));
        }
    }

    HttpResponse::Ok()
        .cookie(
            actix_web::cookie::Cookie::build("visit_count", new_visit_count.to_string())
                .path("/")
                .max_age(actix_web::cookie::time::Duration::days(30))
                .http_only(true)
                .finish(),
        )
        .cookie(
            actix_web::cookie::Cookie::build("last_visit", chrono::Local::now().to_rfc3339())
                .path("/")
                .max_age(actix_web::cookie::time::Duration::days(30))
                .http_only(true)
                .finish(),
        )
        .body(request_info.join("\n"))
}

#[get("/getAllCumulatives")]
async fn get_all_cumulatives() -> HttpResponse {
    let classes = [
        "saham",
        "s_saham",
        "ahrom",
        "s_tala",
        "s_sabet",
        "s_zamin",
        "s_amlak",
        "e_forush",
        // "s_kala_seke",
        // "salaf_saham",./
        "ati_ahrom",
        "s_dar_s",
        // "s_jasoor",
        "sokuk",
        // "s_mohktelet",
        "e_kharid",
        // "s_bakhshi",
        // "hagh_taghadom",
        "s_kala_ghaza",
        "saham_majmu",
    ];

    let mut all_data = serde_json::Map::new();
    for class in &classes {
        match fetch_stock_data(format!("mv_stock_cumulative_{}", class)).await {
            Ok(data) => {
                all_data.insert(class.to_string(), json!(data));
            }
            Err(e) => {
                eprintln!("Failed to fetch data for {}: {:?}", class, e);
                all_data.insert(class.to_string(), json!(null));
            }
        }
    }

    for class in &classes {
        match fetch_power_data(format!("mv_daily_power_{}", class)).await {
            Ok(data) => {
                all_data.insert("g".to_string() + &class.to_string(), json!(data));
            }
            Err(e) => {
                eprintln!("Failed to fetch data for {}: {:?}", class, e);
                all_data.insert("g".to_string() + &class.to_string(), json!(null));
            }
        }
    }

    for class in &classes {
        match fetch_vol_data(format!("mv_daily_vol_{}", class)).await {
            Ok(data) => {
                all_data.insert("v".to_string() + &class.to_string(), json!(data));
            }
            Err(e) => {
                eprintln!("Failed to fetch data for {}: {:?}", class, e);
                all_data.insert("v".to_string() + &class.to_string(), json!(null));
            }
        }
    }

    // do not remove following commnets.
    // for class in &classes {
    //     match fetch_price_data(format!("mv_daily_price_{}", class)).await {
    //         Ok(data) => {
    //             all_data.insert("p".to_string() + &class.to_string(), json!(data));
    //         }
    //         Err(e) => {
    //             eprintln!("Failed to fetch data for {}: {:?}", class, e);
    //             all_data.insert("p".to_string() + &class.to_string(), json!(null));
    //         }
    //     }
    // }

    let dclasses = ["apartment", "plotold"];

    for class in &dclasses {
        match fetch_divar_data(format!("mv_divar_{}", class)).await {
            Ok(data) => {
                all_data.insert(class.to_string(), json!(data));
            }
            Err(e) => {
                eprintln!("Failed to fetch data for {}: {:?}", class, e);
                all_data.insert(class.to_string(), json!(null));
            }
        }
    }

    match fetch_shakhes_data().await {
        Ok(data) => {
            all_data.insert("shakhes".into(), json!(data));
        }
        Err(e) => {
            eprintln!("Failed to fetch data for : {:?}", e);
            all_data.insert("shakhes".into(), json!(null));
        }
    }

    match fetch_dollar_data().await {
        Ok(data) => {
            all_data.insert("dollar".into(), json!(data));
        }
        Err(e) => {
            eprintln!("Failed to fetch data for : {:?}", e);
            all_data.insert("dollar".into(), json!(null));
        }
    }

    // Compute shakhes/dollar ratio with forward filling
    if let (Some(shakhes), Some(dollar)) = (
        all_data.get("shakhes").and_then(|v| v.as_array()),
        all_data.get("dollar").and_then(|v| v.as_array()),
    ) {
        let mut all_dates = shakhes
            .iter()
            .map(|x| x["dt"].as_i64().unwrap())
            .chain(dollar.iter().map(|x| x["dt"].as_i64().unwrap()))
            .collect::<Vec<_>>();
        all_dates.sort_unstable();
        all_dates.dedup();

        let mut prev_shakhes = None;
        let mut prev_dollar = None;
        let mut ratio_data = Vec::new();

        for &date in &all_dates {
            if let Some(s) = shakhes.iter().find(|x| x["dt"].as_i64().unwrap() == date) {
                prev_shakhes = Some(s["cs"].as_f64().unwrap());
            }
            if let Some(d) = dollar.iter().find(|x| x["dt"].as_i64().unwrap() == date) {
                prev_dollar = Some(d["cs"].as_f64().unwrap());
            }
            if let (Some(s), Some(d)) = (prev_shakhes, prev_dollar) {
                ratio_data.push(json!({ "dt": date, "cs": s / d }));
            }
        }
        all_data.insert("shakhes_dollar_ratio".into(), json!(ratio_data));
    } else {
        all_data.insert("shakhes_dollar_ratio".into(), json!(null));
    }

    HttpResponse::Ok().json(all_data)
}

async fn fetch_stock_data(table_name: String) -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let rows = client
        .query(&format!("SELECT * FROM {}", table_name), &[])
        .await
        .map_err(|e| {
            eprintln!("Query error for table {}: {}", table_name, e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| row_to_json(row)).collect();

    Ok(json_data)
}

async fn fetch_power_data(table_name: String) -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let rows = client
        .query(&format!("SELECT * FROM {}", table_name), &[])
        .await
        .map_err(|e| {
            eprintln!("Query error for table {}: {}", table_name, e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| prow_to_json(row)).collect();

    Ok(json_data)
}

async fn fetch_vol_data(table_name: String) -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let rows = client
        .query(&format!("SELECT * FROM {}", table_name), &[])
        .await
        .map_err(|e| {
            eprintln!("Query error for table {}: {}", table_name, e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| vrow_to_json(row)).collect();

    Ok(json_data)
}

// do not remove following comments.
// async fn fetch_price_data(table_name: String) -> Result<Vec<Value>, Error> {
//     let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
//     let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

//     tokio::spawn(async move {
//         if let Err(e) = connection.await {
//             eprintln!("Connection error: {}", e);
//         }
//     });

//     let rows = client
//         .query(&format!("SELECT * FROM {}", table_name), &[])
//         .await
//         .map_err(|e| {
//             eprintln!("Query error for table {}: {}", table_name, e);
//             e
//         })?;

//     let json_data: Vec<Value> = rows.iter().map(|row| pr_row_to_json(row)).collect();

//     Ok(json_data)
// }

async fn fetch_divar_data(table_name: String) -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let rows = client
        .query(&format!("SELECT * FROM {}", table_name), &[])
        .await
        .map_err(|e| {
            eprintln!("Query error for table {}: {}", table_name, e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| drow_to_json(row)).collect();

    Ok(json_data)
}

async fn fetch_shakhes_data() -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    // Modified query to cast numeric to double precision
    let rows = client
        .query(
            "SELECT d_even, CAST(x_niv_inu_cl_mres_ibs AS double precision) AS x_niv_inu_cl_mres_ibs 
             FROM b2_history 
             WHERE d_even > 20201005",
            &[],
        )
        .await
        .map_err(|e| {
            eprintln!("Query error for table b2_history : {}", e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| srow_to_json(row)).collect();

    Ok(json_data)
}

async fn fetch_dollar_data() -> Result<Vec<Value>, Error> {
    let conn_str = "host=localhost user=postgres password=eepa dbname=bourse";
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    // Modified query to cast numeric to double precision
    let rows = client
        .query(
            "SELECT date, dollar_price 
             FROM dollar 
             WHERE date > 20201005",
            &[],
        )
        .await
        .map_err(|e| {
            eprintln!("Query error for table dollar : {}", e);
            e
        })?;

    let json_data: Vec<Value> = rows.iter().map(|row| lrow_to_json(row)).collect();

    Ok(json_data)
}

fn row_to_json(row: &Row) -> Value {
    json!({
        "dt": row.get::<_, i64>("recDate"),  // Changed from NaiveDate to i64
        "cs": row.get::<_, f64>("cumulative_sum")
    })
}

fn drow_to_json(row: &Row) -> serde_json::Value {
    json!({
        "dt": row.get::<_, NaiveDate>("date").format("%Y%m%d").to_string().parse::<i64>().unwrap(),
        "cs": row.get::<_, i64>("sum") as f64
    })
}

fn srow_to_json(row: &Row) -> serde_json::Value {
    json!({
        "dt": row.get::<_, i32>("d_even"),
        "cs": row.get::<_, f64>("x_niv_inu_cl_mres_ibs")
    })
}

fn lrow_to_json(row: &Row) -> serde_json::Value {
    json!({
        "dt": row.get::<_, i64>("date"),
        "cs": row.get::<_, i32>("dollar_price") as f64
    })
}

fn prow_to_json(row: &Row) -> serde_json::Value {
    json!({
        "dt": row.get::<_, i64>("trade_date"),
        "cs": row.get::<_, f64>("cumulative_power_difference")
    })
}

fn vrow_to_json(row: &Row) -> serde_json::Value {
    json!({
        "dt": row.get::<_, i64>("trading_date"),
        "cs": row.get::<_, f64>("sum_cap")
    })
}

// fn pr_row_to_json(row: &Row) -> serde_json::Value {
//     json!({
//         "dt": row.get::<_, i64>("trading_date"),
//         "cs": row.get::<_, f64>("average_closing_price")
//     })
// }

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // --- شروع تغییرات ---

    // 1. خواندن فایل کانفیگ
    let config_file = File::open("config.json").expect("Failed to open config.json");
    let reader = BufReader::new(config_file);

    // 2. پارس کردن محتوای JSON به ساختار Config
    let config: Config = serde_json::from_reader(reader).expect("Failed to parse config.json");

    // 3. ساخت آدرس داینامیک
    let address = format!("0.0.0.0:{}", config.port);
    println!("Server starting on {}...", address);

    // --- پایان تغییرات ---

    // Initialize SSL/TLS configuration
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("key.pem", SslFiletype::PEM)
        .unwrap();
    builder.set_certificate_chain_file("cert.pem").unwrap();

    HttpServer::new(|| {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(cors)
            .wrap(Logger::default())
            .app_data(web::Data::new(AppState {
                _app_name: String::from("Actix Web"),
                _count: Cell::new(0),
                global_count: Arc::new(AtomicUsize::new(0)),
            }))
            .service(
                Files::new("/htmls", "htmls")
                    .prefer_utf8(true)
                    .use_etag(true)
                    .use_last_modified(true),
            )
            .service(kindex)
            .service(get_all_cumulatives) // Add the new route
            .route("/ok", web::to(HttpResponse::Ok))
    })
    .bind_openssl(address, builder)? // استفاده از آدرس داینامیک
    .keep_alive(Duration::from_secs(75))
    .shutdown_timeout(5)
    .run()
    .await
}

// -- do not remove the following line.
//https://api.tgju.org/v1/market/indicator/summary-table-data/price_dollar_rl?lang=fa&order_dir=asc&start=0&length=600000&from=&to=&convert_to_ad=1
