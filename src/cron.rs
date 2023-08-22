use futures::Future;
use std::collections::HashMap;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use uuid::Uuid;

pub struct LivestreamScheduler {
    scheduler: JobScheduler,
    jobs: HashMap<String, Uuid>,
}

type AsyncFn = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;

impl LivestreamScheduler {
    pub async fn new() -> Self {
        let scheduler = JobScheduler::new().await.unwrap();
        scheduler.start().await.unwrap();

        Self {
            scheduler,
            jobs: HashMap::new(),
        }
    }

    pub async fn schedule_livestream_notification(
        &mut self,
        schedule: &str,
        livestream_url: String,
        run: Box<dyn FnMut(Uuid, JobScheduler) -> AsyncFn + Send + Sync>,
    ) -> Result<(), JobSchedulerError> {
        self.cancel_livestream_notification(&livestream_url).await;

        let job_uuid = self.scheduler.add(Job::new_async(schedule, run)?).await?;

        self.jobs.insert(livestream_url, job_uuid);

        Ok(())
    }

    async fn cancel_livestream_notification(&mut self, livestream_url: &str) {
        if let Some(job_uuid) = self.jobs.get(livestream_url) {
            self.scheduler
                .remove(job_uuid)
                .await
                .expect("The job should have been removed");
            self.jobs.remove(livestream_url);
        }
    }
}
