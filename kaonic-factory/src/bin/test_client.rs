use tokio_stream::StreamExt;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

use kaonic::{
    factory_client::FactoryClient, Empty, RunAllTestsRequest, TestStatus,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = FactoryClient::connect("http://127.0.0.1:50011").await?;

    println!("🔧 Factory Test Client");
    println!("======================");

    // Get device info
    println!("\n📱 Getting device info...");
    match client.get_device_info(tonic::Request::new(Empty {})).await {
        Ok(response) => {
            let device_info = response.into_inner();
            println!("✅ Device Serial: {}", device_info.serial);
            println!("✅ Device Machine: {}", device_info.machine);
        }
        Err(e) => {
            println!("❌ Failed to get device info: {}", e);
        }
    }

    // Get available test cases
    println!("\n📋 Getting available test cases...");
    let response = client.get_test_cases(tonic::Request::new(Empty {})).await?;
    let test_cases = response.into_inner().test_cases;

    if test_cases.is_empty() {
        println!("❌ No test cases available");
        return Ok(());
    }

    println!("✅ Found {} test cases:", test_cases.len());
    for test_case in &test_cases {
        println!("   • {} - {}", test_case.id, test_case.name);
        println!("     {}", test_case.description);
    }

    // Run all tests
    println!("\n🚀 Running all tests...");
    println!("========================");

    let mut stream = client
        .run_all_tests(tonic::Request::new(RunAllTestsRequest {}))
        .await?
        .into_inner();

    let mut passed = 0;
    let mut failed = 0;
    let mut total_duration = 0i64;

    while let Some(update) = stream.next().await {
        match update {
            Ok(status) => {
                let status_enum = TestStatus::try_from(status.status).unwrap_or(TestStatus::Failed);
                
                match status_enum {
                    TestStatus::Running => {
                        println!(
                            "\n⏳ [{}/{}] {} - {}",
                            status.current_test,
                            status.total_tests,
                            status.test_id,
                            status.message
                        );
                    }
                    TestStatus::Passed => {
                        passed += 1;
                        total_duration += status.duration_ms;
                        println!(
                            "✅ [{}/{}] {} - {} ({} ms)",
                            status.current_test,
                            status.total_tests,
                            status.test_id,
                            status.message,
                            status.duration_ms
                        );
                    }
                    TestStatus::Failed => {
                        failed += 1;
                        total_duration += status.duration_ms;
                        println!(
                            "❌ [{}/{}] {} - {} ({} ms)",
                            status.current_test,
                            status.total_tests,
                            status.test_id,
                            status.message,
                            status.duration_ms
                        );
                    }
                    _ => {
                        println!(
                            "⚠️  [{}/{}] {} - {} ({} ms)",
                            status.current_test,
                            status.total_tests,
                            status.test_id,
                            status.message,
                            status.duration_ms
                        );
                    }
                }
            }
            Err(e) => {
                println!("❌ Stream error: {}", e);
                break;
            }
        }
    }

    // Summary
    println!("\n📊 Test Summary");
    println!("===============");
    println!("✅ Passed: {}", passed);
    println!("❌ Failed: {}", failed);
    println!("🕒 Total Duration: {} ms", total_duration);
    
    let success_rate = if (passed + failed) > 0 {
        (passed as f64 / (passed + failed) as f64) * 100.0
    } else {
        0.0
    };
    println!("📈 Success Rate: {:.1}%", success_rate);

    if failed == 0 {
        println!("\n🎉 All tests passed!");
    } else {
        println!("\n⚠️  Some tests failed. Please check the results above.");
        std::process::exit(1);
    }

    Ok(())
}
