use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::Instant;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use std::fs;

pub mod kaonic {
    tonic::include_proto!("kaonic");
}

use kaonic::{
    Empty, FactoryTestCaseResponse, RunAllTestsRequest, RunTestRequest, TestCase, TestResult,
    TestStatus, TestStatusUpdate, DeviceInfoResponse, factory_server::Factory,
};

pub mod bluetooth;
pub mod wifi;
pub mod vendor;
pub mod pmic;
pub mod memory;
pub mod i2c;
pub mod rf215;

#[tonic::async_trait]
pub trait FactoryTest: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self) -> Result<String, String>;
}

pub struct FactoryService {
    tests: Arc<HashMap<String, Box<dyn FactoryTest>>>,
}

impl Default for FactoryService {
    fn default() -> Self {
        let mut tests = HashMap::new();
        tests.insert(
            "bluetooth:init".to_string(),
            Box::new(bluetooth::BluetoothInitTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "wifi:init".to_string(),
            Box::new(wifi::WiFiInitTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "vendor:info".to_string(),
            Box::new(vendor::VendorInfoTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "pmic:check".to_string(),
            Box::new(pmic::PmicTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "memory:test".to_string(),
            Box::new(memory::MemoryTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "i2c:devices".to_string(),
            Box::new(i2c::I2cDevicesTest) as Box<dyn FactoryTest>,
        );
        tests.insert(
            "rf215:test".to_string(),
            Box::new(rf215::Rf215Test) as Box<dyn FactoryTest>,
        );

        FactoryService {
            tests: Arc::new(tests),
        }
    }
}

impl FactoryService {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_device_info() -> Result<(String, String), String> {
        let serial = fs::read_to_string("/etc/kaonic/kaonic_serial")
            .map_err(|e| format!("Failed to read serial: {}", e))?
            .trim()
            .to_string();
            
        let machine = fs::read_to_string("/etc/kaonic/kaonic_machine")
            .map_err(|e| format!("Failed to read machine: {}", e))?
            .trim()
            .to_string();
            
        Ok((serial, machine))
    }

    fn get_available_test_cases(&self) -> Vec<TestCase> {
        self.tests
            .iter()
            .map(|(id, test)| TestCase {
                id: id.clone(),
                name: test.name().to_string(),
                description: test.description().to_string(),
            })
            .collect()
    }

    async fn execute_test(&self, test_id: &str) -> TestResult {
        let start_time = Instant::now();

        let (status, message) = match self.tests.get(test_id) {
            Some(test) => match test.execute().await {
                Ok(msg) => (TestStatus::Passed, msg),
                Err(msg) => (TestStatus::Failed, msg),
            },
            None => (
                TestStatus::Failed,
                format!("Unknown test case: {}", test_id),
            ),
        };

        let duration = start_time.elapsed();

        TestResult {
            test_id: test_id.to_string(),
            status: status as i32,
            message,
            duration_ms: duration.as_millis() as i64,
        }
    }
}

#[tonic::async_trait]
impl Factory for FactoryService {
    async fn get_test_cases(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<FactoryTestCaseResponse>, Status> {
        let test_cases = self.get_available_test_cases();
        let response = FactoryTestCaseResponse { test_cases };
        Ok(Response::new(response))
    }

    async fn run_test(
        &self,
        request: Request<RunTestRequest>,
    ) -> Result<Response<TestResult>, Status> {
        let test_id = &request.into_inner().test_id;

        if !self.tests.contains_key(test_id) {
            return Err(Status::not_found(format!(
                "Test case '{}' not found",
                test_id
            )));
        }

        let result = self.execute_test(test_id).await;
        Ok(Response::new(result))
    }

    type RunAllTestsStream = ReceiverStream<Result<TestStatusUpdate, Status>>;

    async fn run_all_tests(
        &self,
        _request: Request<RunAllTestsRequest>,
    ) -> Result<Response<Self::RunAllTestsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let test_cases = self.get_available_test_cases();
        let total_tests = test_cases.len() as i32;

        let tests_clone = Arc::clone(&self.tests);

        tokio::spawn(async move {
            for (index, test_case) in test_cases.iter().enumerate() {
                let current_test = (index + 1) as i32;

                let _ = tx
                    .send(Ok(TestStatusUpdate {
                        test_id: test_case.id.clone(),
                        status: TestStatus::Running as i32,
                        message: format!("Starting {}", test_case.name),
                        duration_ms: 0,
                        current_test,
                        total_tests,
                    }))
                    .await;

                let start_time = Instant::now();
                let (status, message) = match tests_clone.get(&test_case.id) {
                    Some(test) => match test.execute().await {
                        Ok(msg) => (TestStatus::Passed, msg),
                        Err(msg) => (TestStatus::Failed, msg),
                    },
                    None => (TestStatus::Failed, "Test not found".to_string()),
                };

                let duration = start_time.elapsed();

                let _ = tx
                    .send(Ok(TestStatusUpdate {
                        test_id: test_case.id.clone(),
                        status: status as i32,
                        message,
                        duration_ms: duration.as_millis() as i64,
                        current_test,
                        total_tests,
                    }))
                    .await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn get_device_info(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<DeviceInfoResponse>, Status> {
        match Self::read_device_info() {
            Ok((serial, machine)) => {
                let response = DeviceInfoResponse { serial, machine };
                Ok(Response::new(response))
            }
            Err(err) => Err(Status::internal(format!("Failed to read device info: {}", err))),
        }
    }
}
