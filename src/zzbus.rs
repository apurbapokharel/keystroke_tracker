use futures_util::stream::StreamExt;
use zbus::{Connection, proxy};

#[proxy(
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    interface = "org.freedesktop.login1.Manager"
)]
trait Login1Manager {
    // Defines signature for D-Bus signal named `PrepareForSleep`
    #[zbus(signal)]
    fn prepare_for_sleep(&self, status: bool);
}

pub async fn watch_login_jobs() -> anyhow::Result<bool> {
    let connection = Connection::system().await?;
    // `Systemd1ManagerProxy` is generated from `Systemd1Manager` trait
    let systemd_proxy = Login1ManagerProxy::new(&connection).await?;
    // Method `receive_job_new` is generated from `job_new` signal
    let mut new_jobs_stream = systemd_proxy.receive_prepare_for_sleep().await?;

    while let Some(msg) = new_jobs_stream.next().await {
        // struct `JobNewArgs` is generated from `job_new` signal function arguments
        let args: PrepareForSleepArgs = msg.args().expect("Error parsing message");

        println!("Prepare for sleep received : status={}", args.status);
    }

    panic!("Stream ended unexpectedly");
}
