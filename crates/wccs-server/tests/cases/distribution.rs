use super::*;

#[tokio::test]
#[ignore = "requires WCCS_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn resumable_upload_distribution_scope_publication_and_restart() {
    let f = Fixture::new().await;
    let mut ws = socket_for(&f.admin, &f.credential, "device-1").await;
    poll(&mut ws, &observation()).await.unwrap();
    expect_ok(
        f.admin
            .post(
                "configuration",
                &json!({"request_id":token(),"base_version":0,"tools":[],"targets":["device-1"]}),
            )
            .await,
    )
    .await;
    expect_ok(f.admin.post("preparation",&json!({"request_id":token(),"base_version":0,"profile":{"schema_version":1,"tool_version":1,"work_directory":"C:\\Contest","desktop_directory":"C:\\Contest\\Desktop","script_directory":"C:\\Contest\\Scripts","links":[],"package":null}})).await).await;
    let bytes = vec![42u8; wccs_protocol::distribution::CHUNK_SIZE];
    let manifest = json!({"id":"resource","name":"资料.pdf","size":bytes.len()+1,"hashes":[wccs_protocol::bytes_digest(&bytes),wccs_protocol::bytes_digest(b"x")]});
    assert_eq!(
        expect_ok(f.admin.post("distribution/artifacts", &manifest).await).await["uploaded"],
        json!([])
    );
    let send_chunk = |index: usize, body: Vec<u8>| {
        f.admin
            .client
            .post(format!(
                "{}/api/distribution/artifacts/resource/{index}",
                f.admin.url
            ))
            .header("Origin", &f.admin.url)
            .header("Cookie", &f.admin.cookie)
            .header("X-CSRF-Token", &f.admin.csrf)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
    };
    assert_eq!(
        send_chunk(0, b"wrong".to_vec()).await.unwrap().status(),
        400
    );
    assert_eq!(send_chunk(0, bytes.clone()).await.unwrap().status(), 200);
    assert_eq!(send_chunk(0, bytes.clone()).await.unwrap().status(), 200);
    assert_eq!(
        expect_ok(f.admin.post("distribution/artifacts", &manifest).await).await["uploaded"],
        json!([0])
    );
    let mut request = json!({"request_id":token(),"artifact_id":"resource","profile_version":1,"device_ids":["device-1"],"publish_at":wccs_protocol::now()+600,"bytes_per_second":16777216});
    assert_eq!(
        f.admin
            .post("distribution/preview", &request)
            .await
            .status(),
        400
    );
    assert_eq!(send_chunk(1, b"x".to_vec()).await.unwrap().status(), 200);
    expect_ok(
        f.admin
            .device(
                "distribution/poll",
                &f.credential,
                &json!({"adapter":"simulation"}),
            )
            .await,
    )
    .await;
    let preview = expect_ok(f.admin.post("distribution/preview", &request).await).await;
    assert!(preview["targets"][0]["reason"].is_null());
    request["preview_token"] = preview["preview_token"].clone();
    let receipt = expect_ok(f.admin.post("distribution/commit", &request).await).await;
    assert_eq!(
        expect_ok(f.admin.post("distribution/commit", &request).await).await,
        receipt
    );
    let id = receipt["jobs"][0].as_str().unwrap();
    let jobs = expect_ok(
        f.admin
            .device(
                "distribution/poll",
                &f.credential,
                &json!({"adapter":"simulation"}),
            )
            .await,
    )
    .await;
    assert_eq!(jobs["jobs"][0]["publish"], false);
    let download = f
        .admin
        .client
        .get(format!("{}/api/device/distribution/{id}/0", f.admin.url))
        .bearer_auth(&f.credential)
        .send()
        .await
        .unwrap();
    assert_eq!(download.status(), 200);
    assert_eq!(download.bytes().await.unwrap().as_ref(), bytes);
    let mut result =
        json!({"job_id":id,"stage":"published","verified_chunks":2,"detail":"file_published"});
    assert_eq!(
        f.admin
            .device("distribution/progress", &f.credential, &result)
            .await
            .status(),
        400
    );
    result["stage"] = json!("ready");
    result["detail"] = json!("awaiting_publication");
    expect_ok(
        f.admin
            .device("distribution/progress", &f.credential, &result)
            .await,
    )
    .await;
    let restarted = wccs_server::connect(f.app.config.clone()).await.unwrap();
    assert_eq!(
        f.db.query_one("SELECT stage FROM distribution_jobs WHERE id=$1", &[&id])
            .await
            .unwrap()
            .get::<_, String>(0),
        "ready"
    );
    drop(restarted);
    let cancel = json!({"request_id":token(),"job_id":id});
    let cancelled = expect_ok(f.admin.post("distribution/cancel", &cancel).await).await;
    assert_eq!(
        expect_ok(f.admin.post("distribution/cancel", &cancel).await).await,
        cancelled
    );
    assert_eq!(
        f.admin
            .client
            .get(format!("{}/api/device/distribution/{id}/0", f.admin.url))
            .bearer_auth(&f.credential)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        expect_ok(
            f.admin
                .device("distribution/progress", &f.credential, &result)
                .await
        )
        .await["cancelled"],
        true
    );
    request["request_id"] = json!(token());
    request["publish_at"] = json!(0);
    let preview = expect_ok(f.admin.post("distribution/preview", &request).await).await;
    request["preview_token"] = preview["preview_token"].clone();
    let retry = expect_ok(f.admin.post("distribution/commit", &request).await).await;
    let retry_id = retry["jobs"][0].as_str().unwrap();
    f.db.execute(
        "UPDATE devices SET enrollment_generation=enrollment_generation+1 WHERE id='device-1'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        expect_ok(
            f.admin
                .device(
                    "distribution/poll",
                    &f.credential,
                    &json!({"adapter":"simulation"})
                )
                .await
        )
        .await["jobs"],
        json!([])
    );
    assert_eq!(
        f.db.query_one(
            "SELECT stage FROM distribution_jobs WHERE id=$1",
            &[&retry_id]
        )
        .await
        .unwrap()
        .get::<_, String>(0),
        "cancelled"
    );
    f.db.execute(
        "UPDATE operators SET role='viewer' WHERE username='test-admin'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        f.admin
            .post("distribution/artifacts", &manifest)
            .await
            .status(),
        403
    );
    assert_eq!(
        f.admin.post("distribution/commit", &request).await.status(),
        403
    );
    ws.close(None).await.unwrap();
    f.finish().await;
}
