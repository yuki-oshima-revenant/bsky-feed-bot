use aws_lambda_events::eventbridge::EventBridgeEvent;
use lambda_runtime::{service_fn, LambdaEvent};

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    lambda_runtime::run(service_fn(lambda_handler)).await?;
    Ok(())
}

async fn lambda_handler(
    event: LambdaEvent<EventBridgeEvent<serde_json::Value>>,
) -> Result<(), lambda_runtime::Error> {
    println!("{:?}", event.payload.time);
    Ok(())
}
