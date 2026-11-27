use super::*;
use sha2::{Digest, Sha256};
use wccs_protocol::distribution::CHUNK_SIZE;

pub(super) struct Transfers {
    workers: JoinSet<(usize, usize)>,
    started: Instant,
    first_chunk_all_ms: u128,
}

pub(super) async fn start(
    f: &Fixture,
    credentials: Vec<String>,
    phase: Arc<AtomicU8>,
) -> Option<Transfers> {
    if credentials.is_empty() {
        return None;
    }
    let count = credentials.len();
    // Replay downloads of one incompressible, cache-hot block. Seed completed
    // setup before timing so throughput covers the authenticated download route.
    let mut block = vec![0u8; CHUNK_SIZE];
    let (digest_blocks, _) = block.as_chunks_mut::<32>();
    for (index, bytes) in digest_blocks.iter_mut().enumerate() {
        bytes.copy_from_slice(&Sha256::digest((index as u64).to_le_bytes()));
    }
    let hash = wccs_protocol::bytes_digest(&block);
    let manifest = json!({"id":"load-file","name":"load.bin","size":CHUNK_SIZE,"hashes":[hash]});
    f.db.execute(
        "INSERT INTO distribution_artifacts(id,manifest,ready,created_at) VALUES ('load-file',$1,true,0)",
        &[&manifest],
    ).await.unwrap();
    f.db.execute(
        "INSERT INTO distribution_chunks VALUES ('load-file',0,$1)",
        &[&block],
    )
    .await
    .unwrap();
    f.db.execute(
        "INSERT INTO distribution_jobs(id,request_id,actor_id,actor_version,device_id,generation,
            room_ids,artifact_id,directory,adapter,publish_at,bytes_per_second,stage,updated_at)
         SELECT 'load-'||n,'load',o.id,o.version,d.id,d.enrollment_generation,
            ARRAY['room'],'load-file','synthetic','simulation',0,16777216,'downloading',0
         FROM generate_series(1,$1::int) n JOIN devices d ON d.id='device-'||n
         CROSS JOIN operators o WHERE o.username='test-admin'",
        &[&(count as i32)],
    )
    .await
    .unwrap();

    let started = Instant::now();
    let mut workers = JoinSet::new();
    let (ready, mut ready_rx) = mpsc::channel(count);
    for (index, credential) in credentials.into_iter().enumerate() {
        let url = format!(
            "{}/api/device/distribution/load-{}/0",
            f.admin.url,
            index + 1
        );
        let client = f.admin.client.clone();
        let phase = phase.clone();
        let hash = hash.clone();
        let ready = ready.clone();
        workers.spawn(async move {
            let (mut verified, mut retries) = (0, 0);
            while phase.load(Ordering::Relaxed) < 2 {
                let response = client
                    .get(&url)
                    .bearer_auth(&credential)
                    .send()
                    .await
                    .unwrap();
                if response.status() == 429 {
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                assert_eq!(response.status(), 200);
                let bytes = response.bytes().await.unwrap();
                assert_eq!(bytes.len(), CHUNK_SIZE);
                assert_eq!(wccs_protocol::bytes_digest(&bytes), hash);
                verified += 1;
                if verified == 1 {
                    ready.send(()).await.unwrap();
                }
            }
            (verified, retries)
        });
    }
    drop(ready);
    tokio::time::timeout(Duration::from_secs(60), async {
        for _ in 0..count {
            ready_rx
                .recv()
                .await
                .expect("download stopped before control dispatch");
        }
    })
    .await
    .expect("not all downloaders received their first verified chunk");
    Some(Transfers {
        workers,
        started,
        first_chunk_all_ms: started.elapsed().as_millis(),
    })
}

impl Transfers {
    pub(super) async fn finish(mut self) {
        let count = self.workers.len();
        let (mut chunks, mut retries) = (0, 0);
        while let Some(result) = self.workers.join_next().await {
            let (verified, throttled) = result.expect("download worker failed");
            chunks += verified;
            retries += throttled;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        let mib = chunks as f64 * CHUNK_SIZE as f64 / (1024.0 * 1024.0);
        println!(
            "download_load clients={count} first_chunk_all_ms={} verified_chunks={chunks} retries_429={retries} seconds={elapsed:.2} MiB_per_s={:.2}",
            self.first_chunk_all_ms,
            mib / elapsed
        );
    }
}
