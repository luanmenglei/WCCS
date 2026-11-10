use crate::{ApiError, ApiResult, App, auth, management};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wccs_protocol::{now, token, valid_id};

const SUMMARY: &str = "SELECT id,username,display_name,role,enabled,all_rooms,room_ids,version,created_at FROM operators";

fn summary(row: &tokio_postgres::Row) -> Value {
    json!({"id":row.get::<_,String>(0),"username":row.get::<_,String>(1),"display_name":row.get::<_,String>(2),
        "role":row.get::<_,String>(3),"enabled":row.get::<_,bool>(4),"all_rooms":row.get::<_,bool>(5),
        "room_ids":row.get::<_,Vec<String>>(6),"version":row.get::<_,i64>(7),"created_at":row.get::<_,i64>(8)})
}

pub async fn list(State(app): State<App>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, false).await?;
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    let rows = tx
        .query(&format!("{SUMMARY} ORDER BY login_key,id"), &[])
        .await?;
    Ok(Json(json!(rows.iter().map(summary).collect::<Vec<_>>())))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    request_id: String,
    id: String,
    base_version: i64,
    username: String,
    display_name: String,
    role: String,
    enabled: bool,
    all_rooms: bool,
    room_ids: Vec<String>,
    #[serde(default)]
    password: Option<String>,
}

fn validate(input: &Write) -> ApiResult<()> {
    if !valid_id(&input.request_id)
        || !valid_id(&input.id)
        || input.display_name.trim().is_empty()
        || input.display_name.chars().count() > 100
        || input.display_name.chars().any(char::is_control)
        || !["admin", "operator", "viewer"].contains(&input.role.as_str())
        || input.room_ids.len() > 100
        || input.base_version < 0
        || (input.role == "admin" && !input.all_rooms)
        || (input.all_rooms && !input.room_ids.is_empty())
    {
        return Err(ApiError::bad(
            "账号、显示名、角色或机房范围无效，管理员须为全场授权",
        ));
    }
    if input.base_version == 0 {
        let key = auth::login_key(&input.username);
        if !(3..=64).contains(&key.len())
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(ApiError::bad(
                "新账号名为 3～64 位字母、数字、点、横线或下划线，忽略大小写和首尾空白",
            ));
        }
        if input.password.is_none() {
            return Err(ApiError::bad("新账号需要初始密码"));
        }
    }
    if let Some(password) = &input.password
        && (!(12..=256).contains(&password.len()) || password.chars().any(char::is_control))
    {
        return Err(ApiError::bad(
            "新密码需要 12～256 字节，禁止控制字符，空白按原样保留",
        ));
    }
    Ok(())
}

pub async fn save(
    State(app): State<App>,
    headers: HeaderMap,
    Json(mut input): Json<Write>,
) -> ApiResult<Json<Value>> {
    let session = auth::admin(&app, &headers, true).await?;
    validate(&input)?;
    input.room_ids.sort();
    input.room_ids.dedup();
    input.display_name = input.display_name.trim().to_owned();
    if input.base_version == 0 {
        input.username = input.username.trim().to_owned();
    }
    let mut payload = serde_json::to_value(&input).unwrap();
    payload.as_object_mut().unwrap().remove("password");

    // Hash outside the database fence. The final transaction rechecks the session.
    let previous_payload = {
        let mut db = app.db.lock().await;
        let tx = db.transaction().await?;
        auth::authorize(&tx, &session, auth::Permission::Admin).await?;
        tx.query_opt(
            "SELECT payload FROM requests WHERE id=$1",
            &[&input.request_id],
        )
        .await?
        .map(|r| r.get::<_, Value>(0))
    };
    let password_hash = if let Some(password) = input.password.clone() {
        if let Some(hash) = previous_payload
            .as_ref()
            .and_then(|p| p["password_hash"].as_str())
        {
            if !auth::verify(&app, password, hash.to_owned()).await? {
                return Err(ApiError::conflict("请求编号已用于不同内容"));
            }
            Some(hash.to_owned())
        } else {
            let permit = app
                .password_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ApiError::internal())?;
            Some(
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let random = token();
                    let salt = SaltString::encode_b64(&random.as_bytes()[..32])
                        .map_err(|_| ApiError::internal())?;
                    Argon2::default()
                        .hash_password(password.as_bytes(), &salt)
                        .map(|h| h.to_string())
                        .map_err(|_| ApiError::internal())
                })
                .await
                .map_err(|_| ApiError::internal())??,
            )
        }
    } else {
        None
    };
    payload["password_hash"] = json!(password_hash);
    let mut db = app.db.lock().await;
    let tx = db.transaction().await?;
    auth::authorize(&tx, &session, auth::Permission::Admin).await?;
    // Concurrent identical submissions may have independently generated salts.
    // Compare against the committed receipt before applying ordinary replay checks.
    if let Some(password) = input.password
        && let Some(row) = tx
            .query_opt(
                "SELECT payload FROM requests WHERE id=$1",
                &[&input.request_id],
            )
            .await?
        && let Some(hash) = row.get::<_, Value>(0)["password_hash"].as_str()
    {
        if !auth::verify(&app, password, hash.to_owned()).await? {
            return Err(ApiError::conflict("请求编号已用于不同内容"));
        }
        payload["password_hash"] = json!(hash);
    }
    if let Some(response) =
        management::previous(&tx, &input.request_id, "operator_save", &payload).await?
    {
        return Ok(Json(response));
    }
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM rooms WHERE id=ANY($1)",
            &[&input.room_ids],
        )
        .await?
        .get(0);
    if count != input.room_ids.len() as i64 {
        return Err(ApiError::conflict("授权机房已变化，请重新加载"));
    }
    let old = tx
        .query_opt(&format!("{SUMMARY} WHERE id=$1"), &[&input.id])
        .await?;
    let before = old.as_ref().map(summary);
    if let Some(row) = old {
        if row.get::<_, i64>(7) != input.base_version {
            return Err(ApiError::conflict("操作员版本已变化，请重新加载并核对"));
        }
        if row.get::<_, String>(1) != input.username {
            return Err(ApiError::bad("账号名保持稳定，请只修改显示名"));
        }
        if row.get::<_, String>(3) == "admin"
            && row.get::<_, bool>(4)
            && (!input.enabled || input.role != "admin")
        {
            let admins: i64 = tx
                .query_one(
                    "SELECT count(*) FROM operators WHERE enabled AND role='admin'",
                    &[],
                )
                .await?
                .get(0);
            if admins <= 1 {
                return Err(ApiError::conflict("必须保留至少一个启用的管理员"));
            }
        }
        tx.execute("UPDATE operators SET display_name=$2,role=$3,enabled=$4,all_rooms=$5,room_ids=$6,version=version+1,password_hash=COALESCE($7,password_hash) WHERE id=$1",
            &[&input.id,&input.display_name,&input.role,&input.enabled,&input.all_rooms,&input.room_ids,&password_hash]).await?;
    } else {
        if input.base_version != 0 {
            return Err(ApiError::conflict("操作员不存在，请重新加载"));
        }
        let count: i64 = tx
            .query_one("SELECT count(*) FROM operators", &[])
            .await?
            .get(0);
        if count >= 500 {
            return Err(ApiError::bad("最多 500 个操作员账号"));
        }
        if tx
            .query_opt(
                "SELECT id FROM operators WHERE login_key=$1",
                &[&auth::login_key(&input.username)],
            )
            .await?
            .is_some()
        {
            return Err(ApiError::conflict("账号名已存在，大小写和首尾空白视为相同"));
        }
        tx.execute(
            "INSERT INTO operators VALUES($1,$2,$3,$4,$5,$6,$7,$8,1,$9,$10)",
            &[
                &input.id,
                &input.username,
                &auth::login_key(&input.username),
                &input.display_name,
                &input.role,
                &input.enabled,
                &input.all_rooms,
                &input.room_ids,
                &password_hash,
                &now(),
            ],
        )
        .await?;
    }
    let after = summary(
        &tx.query_one(&format!("{SUMMARY} WHERE id=$1"), &[&input.id])
            .await?,
    );
    let response = json!({"request_id":input.request_id,"version":input.base_version+1,"before":before,"operator":after,"credentials_reset":password_hash.is_some()});
    management::record(&tx, &input.request_id, "operator_save", &payload, &response).await?;
    tx.commit().await?;
    app.sessions
        .lock()
        .await
        .retain(|_, session| session.operator_id != input.id);
    Ok(Json(response))
}
