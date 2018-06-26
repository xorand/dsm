use std::collections::HashMap;
use std::error;
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::marker;
use std::net::{TcpListener, TcpStream, Shutdown, SocketAddr, IpAddr, Ipv4Addr};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{SystemTime, Duration};

extern crate hex;
extern crate rand;

extern crate byteorder;
use byteorder::{ByteOrder, BigEndian};

extern crate regex;
use regex::Regex;

#[macro_use]
extern crate serde_derive;
extern crate actix_web;
extern crate serde_urlencoded;
use actix_web::{http, server, App, Form, HttpRequest, HttpResponse, Json, Result};
extern crate uuid;

#[macro_use]
extern crate log;
extern crate chrono;
use chrono::Local;
use chrono::NaiveDateTime;
extern crate log4rs;
extern crate log_panics;

#[macro_use]
extern crate lazy_static;

use log::LevelFilter;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
use log4rs::append::rolling_file::RollingFileAppender;
use log4rs::config::{Appender, Config, Root};
use log4rs::encode::pattern::PatternEncoder;

extern crate ini;
use ini::Ini;

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_derive_enum;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

#[macro_use]
extern crate askama;
use askama::Template;

static PRG: &'static str = "dsm";

#[derive(Debug, PartialEq, DbEnum)]
pub enum MsgType {
    SmsIn,
    SmsOut,
    UssdIn,
    UssdOut,
}

impl fmt::Display for MsgType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MsgType::SmsIn => write!(f, "sms in"),
            MsgType::SmsOut => write!(f, "sms out"),
            MsgType::UssdIn => write!(f, "ussd in"),
            MsgType::UssdOut => write!(f, "ussd out"),
        }
    }
}

#[derive(Debug, PartialEq, DbEnum)]
pub enum MsgStatus {
    Sending,
    Sent,
}

impl fmt::Display for MsgStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MsgStatus::Sending => write!(f, "sending"),
            MsgStatus::Sent => write!(f, "sent"),
        }
    }
}

#[derive(Queryable, Insertable, Identifiable)]
#[table_name = "msg"]
#[primary_key(msg_id)]
pub struct Msg {
    phone: String,
    msg_txt: String,
    msg_id: String,
    msg_date: NaiveDateTime,
    msg_type: MsgType,
    slot: i32,
    status: MsgStatus,
}

table! {
    use diesel::sql_types::Integer;
    use diesel::sql_types::Text;
    use diesel::sql_types::Timestamp;
    use super::MsgTypeMapping;
    use super::MsgStatusMapping;
    msg (msg_id) {
        phone -> Text,
        msg_txt -> Text,
        msg_id -> Text,
        msg_date -> Timestamp,
        msg_type -> MsgTypeMapping,
        slot -> Integer,
        status -> MsgStatusMapping,
    }
}

fn read_cfg() -> AppCfg {
    let ini = Ini::load_from_file(format!("{}.ini", PRG)).unwrap_or(Ini::new());
    let cfg = ini.section(Some(PRG)).unwrap_or(&HashMap::new()).to_owned();
    let app_cfg = AppCfg {
        log_level: cfg.get("log_level")
            .unwrap_or(&"INFO".to_string())
            .parse::<LevelFilter>()
            .unwrap_or(LevelFilter::Info),
        log_size: cfg.get("log_size")
            .unwrap_or(&"1048576".to_string())
            .parse::<u64>()
            .unwrap_or(1048576),
        log_num: cfg.get("log_num")
            .unwrap_or(&"2".to_string())
            .parse::<u32>()
            .unwrap_or(2),
        log_console: cfg.get("log_console")
            .unwrap_or(&"true".to_string())
            .parse::<bool>()
            .unwrap_or(true),
        api_key: cfg.get("api_key").unwrap_or(&"api".to_string()).to_string(),
        api_host: cfg.get("api_host")
            .unwrap_or(&"0.0.0.0".to_string())
            .to_string(),
        api_port: cfg.get("api_port")
            .unwrap_or(&"8000".to_string())
            .to_string(),
        gw_port: cfg.get("gw_port")
            .unwrap_or(&"9000".to_string())
            .to_string(),
        gw_ping_timer: cfg.get("gw_ping_timer")
            .unwrap_or(&"60".to_string())
            .parse::<u16>()
            .unwrap_or(60),
        gw_queue_timer: cfg.get("gw_queue_timer")
            .unwrap_or(&"5".to_string())
            .parse::<u16>()
            .unwrap_or(5),
    };
    app_cfg
}

fn setup_log() -> Result<(), Box<error::Error + marker::Sync + marker::Send>> {
    log_panics::init();
    let pattern = "{d([%d-%m-%Y][%H:%M:%S])}[{l}][{M}]{m}\n";
    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(pattern)))
        .build();
    let roller = FixedWindowRoller::builder()
        .build(&format!("{}.{{}}.log", PRG), APP_STATE.app_cfg.log_num)?;
    let policy = CompoundPolicy::new(
        Box::new(SizeTrigger::new(APP_STATE.app_cfg.log_size)),
        Box::new(roller),
    );
    let log = RollingFileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(pattern)))
        .build(format!("{}.log", PRG), Box::new(policy))?;
    let config;
    if APP_STATE.app_cfg.log_console {
        config = Config::builder()
            .appender(Appender::builder().build("stdout", Box::new(stdout)))
            .appender(Appender::builder().build("log", Box::new(log)))
            .build(
                Root::builder()
                    .appenders(vec!["stdout", "log"])
                    .build(APP_STATE.app_cfg.log_level),
            )?;
    } else {
        config = Config::builder()
            .appender(Appender::builder().build("log", Box::new(log)))
            .build(
                Root::builder()
                    .appender("log")
                    .build(APP_STATE.app_cfg.log_level),
            )?;
    }
    log4rs::init_config(config)?;
    Ok(())
}

fn conn_db() -> Result<SqliteConnection, Box<error::Error>> {
    let db = SqliteConnection::establish(&format!("{}.sqlite", PRG))?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS msg(
        phone TEXT,
        msg_txt TEXT,
        msg_id TEXT,
        msg_date TXT,
        msg_type TEXT CHECK(msg_type IN ('sms_in', 'sms_out', 'ussd_in', 'ussd_out')) NOT NULL,
        slot INT,
        status TEXT CHECK(status IN ('sending', 'sent')) NOT NULL)",
    )?;
    db.execute("CREATE INDEX IF NOT EXISTS msg_id ON msg (msg_id)")?;
    Ok(db)
}

#[derive(Template)]
#[template(
    source = "<html>
    gw status:<font color='{{color}}'>{{status}}</font><br>
    <a href='/send_sms/'>send sms</a><br>
    <a href='/send_ussd/'>send ussd</a><br>
    <a href='/msgs/'>msg base</a><br>
    </form>
    </html>",
    ext = "html"
)]
struct WebRootTpl<'a> {
    color: &'a str,
    status: &'a str,
}

fn web_root(_: HttpRequest) -> HttpResponse {
    let color;
    let status_txt;
    let mut body = "".to_string();
    if let Ok(status) = APP_STATE.status.read() {
        if *status {
            color = "green";
            status_txt = "alive";
        } else {
            color = "red";
            status_txt = "dead";
        }
        body = WebRootTpl {
            color: color,
            status: status_txt,
        }.render()
            .unwrap_or("".to_string());        
    } else {
        info!("error acquiring lock on read status");
    }
    HttpResponse::Ok().content_type("text/html").body(body)
}

#[derive(Template)]
#[template(
    source = "<html>
    <form accept-charset='utf-8' action='' method='post'>
        <table>
        {% if send_type == WebSendType::Sms %}
        <tr><td>phone</td><td><input name='phone' type='text' size='10'></td></tr>
        <tr><td>sms text</td><td><textarea name='sms' cols='40' rows='3'></textarea></td></tr>
        {% else %}
        <tr><td>ussd</td><td><input name='ussd' type='text' size='10'></td></tr>
        {% endif %}
        <tr><td></td><td><input type='radio' name='slot' value='1' checked>slot #1</td></tr>
        <tr><td></td><td><input type='radio' name='slot' value='2'>slot #2</td></tr>
        <tr><td></td><td><input type='submit' value='send'/></td></tr>
    </form>
    </html>",
    ext = "html"
)]
struct WebSendTpl {
    send_type: WebSendType,
}
#[derive(PartialEq)]
enum WebSendType {
    Sms = 1,
    Ussd = 2,
}

fn web_send_sms_form(_: HttpRequest) -> HttpResponse {
    let body = WebSendTpl {
        send_type: WebSendType::Sms,
    }.render()
        .unwrap_or("".to_string());
    HttpResponse::Ok().content_type("text/html").body(body)
}

#[derive(Deserialize)]
struct SmsInfo {
    phone: String,
    sms: String,
    slot: i32,
}

fn web_send_sms(form: Form<SmsInfo>) -> HttpResponse {
    let phone: String = RE_NUM.replace_all(&form.phone, "").to_owned().to_string();
    if let Err(e) = add_msg(MsgType::SmsOut, &phone, &form.sms, form.slot) {
        info!("error adding sms to database: {:?}", e);
    }
    HttpResponse::MovedPermanenty()
        .header("Location", "/")
        .body("")
}

fn web_send_ussd_form(_: HttpRequest) -> HttpResponse {
    let body = WebSendTpl {
        send_type: WebSendType::Ussd,
    }.render()
        .unwrap_or("".to_string());
    HttpResponse::Ok().content_type("text/html").body(body)
}

#[derive(Deserialize)]
struct UssdInfo {
    ussd: String,
    slot: i32,
}

fn web_send_ussd(form: Form<UssdInfo>) -> HttpResponse {
    let ussd: String = RE_USSD.replace_all(&form.ussd, "").to_owned().to_string();
    if let Err(e) = add_msg(MsgType::UssdOut, "", &ussd, form.slot) {
        info!("error adding ussd to database: {:?}", e);
    };
    HttpResponse::MovedPermanenty()
        .header("Location", "/")
        .body("")
}

fn add_msg(
    msg_type: MsgType,
    phone: &str,
    sms: &str,
    slot: i32,
) -> Result<(String), Box<error::Error>> {
    let db = conn_db()?;
    let msg_id = format!("{}", uuid::Uuid::new_v4().simple());
    let msg_val = Msg {
        phone: phone.to_string(),
        msg_txt: sms.to_string(),
        msg_id: msg_id.clone(),
        msg_date: Local::now().naive_local(),
        msg_type: msg_type,
        slot: slot,
        status: MsgStatus::Sending,
    };
    diesel::insert_into(msg::table)
        .values(&msg_val)
        .execute(&db)?;
    Ok(msg_id)
}

#[derive(Template)]
#[template(
    source = "<html>
    <meta charset='utf-8'>
    <table border=1>
    <th>date</th><th>msg type</th><th>phone</th><th>msg id</th><th>msg</th><th>slot</th><th>status</th>
    {% for msg in msgs %}
    <tr><td>{{msg.msg_date}}</td>
        <td>{{msg.msg_type}}</td>
        <td>{{msg.phone}}</td>
        <td>{{msg.msg_id}}</td>
        <td>{{msg.msg_txt}}</td>
        <td>{{msg.slot}}</td>
        <td>{{msg.status}}</td>
    </tr>
    {% endfor %}
    </table></html>",
    ext = "html"
)]
struct WebMsgsTpl {
    msgs: Vec<Msg>,
}

fn web_msgs(_: HttpRequest) -> HttpResponse {
    if let Ok(db) = conn_db() {
        if let Ok(msgs) = msg::table.load::<Msg>(&db) {
            let body = WebMsgsTpl { msgs: msgs }.render().unwrap_or("".to_string());
            return HttpResponse::Ok().content_type("text/html").body(body);
        } else {
            info!("error loading database messages");
        }
    } else {
        info!("error connecting to database while display message base");
    }
    HttpResponse::Ok().into()
}

#[derive(Deserialize)]
struct ApiSendInfo {
    cmd: String,
    api_key: String,
    message: String,
    to: String,
}

#[derive(Deserialize)]
struct ApiCheckInfo {
    cmd: String,
    api_key: String,
    sms_id: String,
}

#[derive(Serialize)]
struct ApiResult {
    error_no: u8,
    error_msg: String,
    items: HashMap<String, String>,
}

fn api_check_sms(sms_id: String) -> bool {
    if let Ok(db) = conn_db() {
        if let Ok(results) = msg::table.filter(msg::msg_id.eq(sms_id)).load::<Msg>(&db) {
            let mut status = false;
            for result in results {
                if result.status == MsgStatus::Sent {
                    status = true;
                }
            }
            status
        } else {
            info!("error loading check sms status from database");
            false
        }
    } else {
        info!("error connecting to database while check sms status");
        false
    }
}

macro_rules! check_api_key {
    ($info:ident) => {{
        if $info.api_key != APP_STATE.app_cfg.api_key {
            return Ok(Json(ApiResult {
                error_no: 1,
                error_msg: "API Key Not Found".to_string(),
                items: HashMap::new(),
            }));
        }
    }};
}

fn api(req: String) -> Result<Json<ApiResult>> {
    if let Ok(info) = serde_urlencoded::from_str::<ApiSendInfo>(&req) {
        check_api_key!(info);
        if (info.cmd == "send") && (info.message != "") && (info.to != "") {
            let to_n: String = RE_NUM.replace_all(&info.to, "").to_owned().to_string();
            if let Ok(sms_id) = add_msg(MsgType::SmsOut, &to_n, &info.message, 1) {
                return Ok(Json(ApiResult {
                    error_no: 0,
                    error_msg: "OK".to_string(),
                    items: [
                        ("phone".to_string(), to_n),
                        ("sms_id".to_string(), sms_id),
                        ("error_no".to_string(), "0".to_string()),
                        ("error_msg".to_string(), "OK".to_string()),
                    ].iter()
                        .cloned()
                        .collect(),
                }));
            } else {
                info!("error adding msg from api call");
            }
        }
    };
    if let Ok(info) = serde_urlencoded::from_str::<ApiCheckInfo>(&req) {
        check_api_key!(info);
        if (info.cmd == "status") && (info.sms_id != "") {
            let status;
            if api_check_sms(info.sms_id) {
                status = "2";
            } else {
                status = "10";
            };
            return Ok(Json(ApiResult {
                error_no: 0,
                error_msg: "OK".to_string(),
                items: [
                    ("status_no".to_string(), status.to_string()),
                    ("error_msg".to_string(), "OK".to_string()),
                ].iter()
                    .cloned()
                    .collect(),
            }));
        }
    };
    Ok(Json(ApiResult {
        error_no: 0,
        error_msg: "OK".to_string(),
        items: HashMap::new(),
    }))
}

struct AppState {
    status: Arc<RwLock<bool>>,
    app_cfg: Arc<AppCfg>,
}

struct AppCfg {
    log_level: LevelFilter,
    log_size: u64,
    log_num: u32,
    log_console: bool,
    api_key: String,
    api_host: String,
    api_port: String,
    gw_port: String,
    gw_ping_timer: u16,
    gw_queue_timer: u16,
}

lazy_static! {
    static ref APP_STATE: AppState = AppState {
        status: Arc::new(RwLock::new(false)),
        app_cfg: Arc::new(read_cfg()),
    };
}

lazy_static! {
    static ref RE_NUM: Regex = Regex::new(r"[^\d]+").expect("could not compile regex");
    static ref RE_USSD: Regex = Regex::new(r"[^\d*#]+").expect("could not compile regex");
}

#[derive(Debug)]
struct DinHeader {
    len: u32,
    mac: String,
    time: u32,
    serial: u32,
    htype: u16,
    flag: u16,
}

#[derive(Debug)]
struct DinData {
    htype: u16,
    body: Vec<u8>,
}

#[derive(Debug)]
struct DinSms {
    number: String,
    stype: u8,
    port: u8,
    timestamp: String,
    timezone: i8,
    encoding: u8,
    length: u16,
    content: String,
}

#[derive(Debug)]
struct DinUssd {
    port: u8,
    status: u8,    
    length: u16,
    encoding: u8,
    content: String,
}

fn gw_save_sms(sms: DinSms) -> Result<(), Box<error::Error>>{
    let db = conn_db()?;
    let msg_id = format!("{}", uuid::Uuid::new_v4().simple());
    let msg_val = Msg {
        phone: sms.number,
        msg_txt: sms.content,
        msg_id: msg_id.clone(),
        msg_date: NaiveDateTime::parse_from_str(&sms.timestamp, "%Y%m%d%H%M%S")
            .unwrap_or(Local::now().naive_local()),
        msg_type: MsgType::SmsIn,
        slot: sms.port as i32,
        status: MsgStatus::Sent,
    };
    diesel::insert_into(msg::table)
        .values(&msg_val)
        .execute(&db)?;
    Ok(())
}

fn gw_save_ussd(ussd: DinUssd) -> Result<(), Box<error::Error>>{
    let db = conn_db()?;
    let msg_id = format!("{}", uuid::Uuid::new_v4().simple());
    let msg_val = Msg {
        phone: "".to_string(),
        msg_txt: ussd.content,
        msg_id: msg_id.clone(),
        msg_date: Local::now().naive_local(),
        msg_type: MsgType::UssdIn,
        slot: ussd.port as i32,
        status: MsgStatus::Sent,
    };
    diesel::insert_into(msg::table)
        .values(&msg_val)
        .execute(&db)?;
    Ok(())
}

fn gw_get_content(v8: Vec<u8>, encoding: u8) -> String {
    if encoding == 1 {
        let mut v16: Vec<u16> = Vec::new();
        for i in 0..v8.len()/2 {
            let u16n =((v8[i * 2] as u16) << 8) | v8[i * 2 + 1] as u16;
            v16.push(u16n);
        }
        String::from_utf16_lossy(&v16)
    } else {
        String::from_utf8_lossy(&v8).to_string()
    }
}

fn gw_parse_type(htype: u16, data: &[u8], ping: &mut Ping) -> DinData {
    match htype {
        0 => { // ping alive
            ping.ping_sent = false;
            if let Ok(mut status) = APP_STATE.status.write() {
                *status = true;
            } else {
                info!("error when acquring lock write on status");    
            }
            info!("<- gw alive");
        }
        7 => { // status message
            info!("<- status message");
            return DinData {
                htype: 8,
                body: vec!(0),
            }
        }
        5 => { // receive message
            info!("<- receive message");
            let mut sms = DinSms {
                number: String::from_utf8_lossy(&data[0..24]).to_string().replace("\u{0}",""),
                stype: data[24],
                port: data[25] + 1,
                timestamp: String::from_utf8_lossy(&data[26..41]).to_string().replace("\u{0}",""),
                timezone: data[41] as i8,
                encoding: data[42],
                length: BigEndian::read_u16(&data[43..45]),
                content: "".to_string(),
            };
            sms.content = gw_get_content(data[45..].to_vec(), sms.encoding);
            if let Err(e) = gw_save_sms(sms) {
                info!("error when saving sms to database: {}", e);
            }
            return DinData {
                htype: 6,
                body: vec!(0),
            }
        }
        3 => { // sms result
            info!("<- sms result");
            return DinData {
                htype: 4,
                body: vec!(0),
            }
        }
        11 => { // ussd result
            info!("<- ussd result");
            let mut ussd = DinUssd {
                port: data[0] + 1,
                status: data[1],
                length: BigEndian::read_u16(&data[2..4]),
                encoding: data[4],                
                content: "".to_string(),
            };  
            let content = gw_get_content(data[5..].to_vec(), ussd.encoding);
            let content_bin = hex::decode(content).unwrap_or(vec!(0));
            ussd.content = gw_get_content(content_bin, 1);
            if let Err(e) = gw_save_ussd(ussd) {
                info!("error when saving ussd to database: {}", e);
            }
            return DinData {
                htype: 12,
                body: vec!(0),
            }
        }
        515 => { // call state report
            info!("<- call state result");
            return DinData {
                htype: 516,
                body: vec!(0),
            }
        }
        _ => {}
    } 
    DinData {
        htype: 0,
        body: vec!(),
    }
}

fn pkt_u32(pkt: &mut Vec<u8>, value: u32) {
    let mut buf: Vec<u8> = vec![0; 4];
    BigEndian::write_u32(&mut buf, value);
    pkt.append(&mut buf);
}

fn pkt_u16(pkt: &mut Vec<u8>, value: u16) {
    let mut buf: Vec<u8> = vec![0; 2];
    BigEndian::write_u16(&mut buf, value);
    pkt.append(&mut buf);
}

fn gw_send(mut stream: &TcpStream, header: DinHeader, sdata: DinData) {
    let mut pkt: Vec<u8> = vec!();
    pkt_u32(&mut pkt, sdata.body.len() as u32);
    pkt.append(&mut Vec::from(hex::decode(header.mac).unwrap_or(vec!(0,0,0,0,0,0))));
    pkt_u16(&mut pkt, 0);
    pkt_u32(&mut pkt, header.time);
    pkt_u32(&mut pkt, header.serial);
    pkt_u16(&mut pkt, sdata.htype);
    pkt_u16(&mut pkt, 0);
    pkt.append(&mut Vec::from(sdata.body));
    info!("-> {}", hex::encode(&pkt));
    if let Err(e) = stream.write(&pkt) {
        info!("error when writing to socket: {}", e);
    }
}

fn gw_parse_data(stream: &TcpStream, data: &[u8], ping: &mut Ping) {
    if data.len() < 24 {()}
    info!("<- {}", hex::encode(data));
    let header = DinHeader {
        len: BigEndian::read_u32(&data[0..4]),
        mac: hex::encode(&data[4..10]),
        time: BigEndian::read_u32(&data[12..16]),
        serial: BigEndian::read_u32(&data[16..20]),
        htype: BigEndian::read_u16(&data[20..22]),
        flag: BigEndian::read_u16(&data[22..24]),
    };
    let data_len: usize = 24 + (header.len as usize);
    if data.len() < data_len {()}
    let sdata = gw_parse_type(header.htype, &data[24..data_len], ping);
    if sdata.htype != 0 {
        gw_send(stream, header, sdata)
    }
}

fn gw_create_header() -> DinHeader {
    DinHeader {
        len: 0,
        mac: "00fab3d2d3aa".to_string(),
        time: Local::now().timestamp() as u32,
        serial: rand::random::<u32>(),
        htype: 0,
        flag: 0,
    }
}   

fn gw_ping_fn(stream: &TcpStream) {
    gw_send(
        stream,
        gw_create_header(),
        DinData {
            htype: 0,
            body: vec!(),
        })
}  

fn gw_queue_fn(stream: &TcpStream) {
    if let Ok(db) = conn_db() {
        if let Ok(results) = msg::table.filter(msg::status.eq(MsgStatus::Sending)).load::<Msg>(&db) {
            for result in results {
                match result.msg_type {
                    MsgType::SmsOut => {
                        info!("sending sms to number {}", result.phone);
                        let mut body: Vec<u8> = Vec::new();
                        body.push(result.slot as u8 - 1);
                        body.push(1);
                        body.push(0);
                        body.push(1);
                        body.push(43); // +
                        let mut phone_bytes = result.phone.as_bytes().to_vec();
                        body.append(&mut phone_bytes);
                        for _ in 1 .. (13 - phone_bytes.len()) {
                            body.push(0);
                        }
                        let mut msg: Vec<u8> = Vec::new();
                        let mut v16: Vec<u16> = result.msg_txt.encode_utf16().collect();
                        for v in v16 {
                            let mut v8: Vec<u8> = vec!(0, 0);
                            BigEndian::write_u16(&mut v8, v);
                            msg.append(&mut v8);
                        }
                        let mut len: Vec<u8> = vec!(0, 0);
                        BigEndian::write_u16(&mut len, msg.len() as u16);
                        body.append(&mut len);
                        body.append(&mut msg);
                        let sdata = DinData {
                            htype: 1,
                            body: body,
                        };
                        gw_send(stream, gw_create_header(), sdata);
                    },
                    MsgType::UssdOut => {
                        info!("sending ussd to port {}", result.slot);
                        let mut body: Vec<u8> = Vec::new();
                        body.push(result.slot as u8 - 1);
                        body.push(1);
                        body.push(0);
                        body.push(0);                        
                        BigEndian::write_u16(&mut body[2..4], result.msg_txt.len() as u16);
                        let mut msg_bytes = result.msg_txt.as_bytes().to_vec();
                        body.append(&mut msg_bytes);
                        let sdata = DinData {
                            htype: 9,
                            body: body,
                        };
                        gw_send(stream, gw_create_header(), sdata);
                    },
                    _ => {}
                }
                let target = msg::table.filter(msg::msg_id.eq(result.msg_id));
                if let Err(e) = diesel::update(target)
                    .set(msg::status.eq(MsgStatus::Sent))
                    .execute(&db) {
                    info!("error when updating status of msg in database: {}", e);
                }
            }
        } else {
            info!("error loading smses to send from database");
        }
    } else {
        info!("error connecting to database while sending smses");
    }
}

fn gw_conn(mut stream: TcpStream, peer_addr: SocketAddr) {
    let mut data = [0 as u8; 66560];
    let mut now = SystemTime::now();
    let mut ping = Ping { ping_sent: false };
    stream.set_read_timeout(Some(Duration::from_secs(APP_STATE.app_cfg.gw_queue_timer as u64)))
        .expect("could not set stream read timeout");
    while match stream.read(&mut data) {
        Ok(size) => {
            if size != 0 {
                gw_parse_data(&stream, &data[0..size], &mut ping);
                true
            }
            else {
                false
            }
        },
        Err(e) => {
            if e.kind() != ErrorKind::TimedOut {
                info!("error occurred, terminating connection with {}", peer_addr);
                if let Err(e) = stream.shutdown(Shutdown::Both) {
                    info!("error when shutdown stream with {} :{}", peer_addr, e);
                }
                false
            } else {                
                if let Ok(dur) = now.elapsed() {
                    if dur > Duration::from_secs(APP_STATE.app_cfg.gw_ping_timer as u64) {
                        if ping.ping_sent {
                            ping.ping_sent = false;
                            info!("error when pinging gw, shutdown stream");
                            if let Err(e) = stream.shutdown(Shutdown::Both) {
                                info!("error when shutdown stream with {} :{}", peer_addr, e);
                            }
                            if let Ok(mut status) = APP_STATE.status.write() {
                                *status = false;
                            } else {
                                info!("error when acquring lock write on status");    
                            }
                        } else {
                            ping.ping_sent = true;
                            gw_ping_fn(&stream);                            
                            now = SystemTime::now();
                        }
                    }
                }
                gw_queue_fn(&stream);
                true
            }
        }
    } {}
    info!("exiting gw thread");
}

struct Ping {
    ping_sent: bool,
}

fn gw_th_fn() {
    let bind = &format!(
        "{}:{}",
        APP_STATE.app_cfg.api_host, APP_STATE.app_cfg.gw_port
    );
    if let Ok(listener) = TcpListener::bind(bind) {
        info!("gw listener start on {}", bind);
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let peer_addr = stream.peer_addr().unwrap_or(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0));
                    info!("gw connected: {}", peer_addr);
                    if let Ok(mut status) = APP_STATE.status.write() {
                        *status = true;
                    } else {
                        info!("error when acquring lock write on status");    
                    }
                    thread::spawn(move|| {
                        gw_conn(stream, peer_addr)
                    });
                }
                Err(e) => {
                    info!("error when connecting gw: {}", e);
                }
            }
        }
        drop(listener);
    } else {
        info!("can not bind gw listener to {}", bind);
    }
    
}

fn main() {
    setup_log().expect("can not setup logging");    
    let bind = &format!(
        "{}:{}",
        APP_STATE.app_cfg.api_host, APP_STATE.app_cfg.api_port
    );
    thread::spawn(|| {
        gw_th_fn();
    });
    server::new(|| {
        App::new()
            .resource("/api/", |r| r.method(http::Method::POST).with(api))
            .resource("/", |r| r.f(web_root))
            .resource("/msgs/", |r| r.f(web_msgs))
            .resource("/send_sms/", |r| {
                r.method(http::Method::GET).f(web_send_sms_form);
                r.method(http::Method::POST).with(web_send_sms);
            })
            .resource("/send_ussd/", |r| {
                r.method(http::Method::GET).f(web_send_ussd_form);
                r.method(http::Method::POST).with(web_send_ussd);
            })
    }).bind(bind)
        .expect(&format!("can not bind api to {}", bind))
        .run();
}