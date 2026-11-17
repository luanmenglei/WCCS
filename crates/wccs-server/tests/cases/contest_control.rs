use super::*;

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn contest_schedule_cancel_replay_and_restart() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    let input = json!({"request_id":token(),"contest_id":"contest","remote_contest_id":"demo","seat_ids":["seat-1"],"action":"pause","execute_at":0,"duration_seconds":900});
    assert_eq!(
        f.admin
            .post("contest-control/preview", &input)
            .await
            .status(),
        409
    );
    let actor: String =
        f.db.query_one("SELECT id FROM operators WHERE role='admin'", &[])
            .await
            .unwrap()
            .get(0);
    let insert = "INSERT INTO contest_control_jobs(id,actor_id,actor_version,stage,execute_at,payload,plan,created_at,updated_at) VALUES($1,$2,1,'scheduled',9999999999,$3,'{}',0,0)";
    f.db.execute(insert, &[&"cancel-probe", &actor, &input])
        .await
        .unwrap();
    let cancel = json!({"request_id":token(),"job_id":"cancel-probe"});
    let result = expect_ok(f.admin.post("contest-control/cancel", &cancel).await).await;
    assert_eq!(result["stage"], "cancelled");
    assert_eq!(
        expect_ok(f.admin.post("contest-control/cancel", &cancel).await).await,
        result
    );
    assert_eq!(
        f.admin
            .post(
                "contest-control/cancel",
                &json!({"request_id":token(),"job_id":"cancel-probe"})
            )
            .await
            .status(),
        409
    );
    f.db.execute(insert, &[&"restart-probe", &actor, &input])
        .await
        .unwrap();
    let restarted = wccs_server::connect(f.app.config.clone()).await.unwrap();
    assert_eq!(
        f.db.query_one(
            "SELECT stage FROM contest_control_jobs WHERE id='restart-probe'",
            &[]
        )
        .await
        .unwrap()
        .get::<_, String>(0),
        "interrupted"
    );
    assert_eq!(
        f.db.query_one(
            "SELECT stage FROM contest_control_jobs WHERE id='cancel-probe'",
            &[]
        )
        .await
        .unwrap()
        .get::<_, String>(0),
        "cancelled"
    );
    drop(restarted);
    drop(ws);
    f.finish().await;
}
