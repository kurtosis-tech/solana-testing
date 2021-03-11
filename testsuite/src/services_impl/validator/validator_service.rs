use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use kurtosis_rust_lib::services::{service::Service, service_context::ServiceContext};
use serde_json::{Value, json};

use super::validator_container_initializer::TEST_VOLUME_MOUNTPOINT;
use super::{http_sender::HttpSender, rpc_request::RpcRequest, rpc_sender::RpcSender};

pub (super) const RPC_PORT: u32 = 8899;
pub (super) const GOSSIP_PORT: u32 = 8001;
const GOSSIP_CLI_GOSSIP_PORT: u32 = 9000;
const TIMEOUT: Duration = Duration::from_secs(60);
const JSON_CONTENT_TYPE: &str = "application/json";
const GET_VERSION_RPC_REQUEST: &str = "{\"jsonrpc\":\"2.0\",\"id\":1, \"method\":\"getVersion\"}";

pub (super) const INIT_COMPLETE_FILEPATH: &str = "/tmp/init-complete.log";

const SOLANA_BINARIES_DIRPATH: &str = "/usr/bin";
const SOLANA_CLI_BIN_FILENAME: &str = "solana";
const SOLANA_KEYGEN_BIN_FILENAME: &str = "solana-keygen";
const SOLANA_GOSSIP_BIN_FILENAME: &str = "solana-gossip";

const COMMITMENT_LEVEL_PARAM: &str = "commitment";
const CONFIRMED_COMMMITMENT_LEVEL: &str = "confirmed";

const SUCCESSFUL_EXIT_CODE: i32 = 0;

pub struct ValidatorService {
    service_context: ServiceContext,
    sender: Box<dyn RpcSender>,
}

impl ValidatorService {
    pub fn new(service_context: ServiceContext) -> ValidatorService {
        let url = format!("http://{}:{}", service_context.get_ip_address(), RPC_PORT);
        return ValidatorService{
            service_context,
            sender: Box::new(HttpSender::new(url)),
        };
    }

    pub fn get_ip_address(&self) -> &str {
        return self.service_context.get_ip_address();
    }

    // TODO All of the methods below this point can be replaced by the official Solana RpcClient:
    // https://github.com/solana-labs/solana/blob/master/client/src/rpc_client.rs
    // Unfortunately, that library (solana-client) provides Ledger support, and so depends on the 'hidapi'
    // kernel module. This won't pass compilation on Docker-for-Mac due to:
    // https://github.com/docker/for-mac/issues/5295
    // Until either:
    //   a) solana-client makes Ledger dependencies optional or
    //   b) Docker-for-Mac supports Linux headers
    // we have to reimplement the client methods
    pub fn get_confirmed_transaction_count(&self) -> Result<u64> {
        let params = json!([
            {
                COMMITMENT_LEVEL_PARAM: CONFIRMED_COMMMITMENT_LEVEL,
            },
        ]);
        let result = self.send(RpcRequest::GetTransactionCount, params)
            .context("An error occurred getting the confirmed transaction count")?;
        return Ok(result);
    }

    pub fn assert_number_of_nodes(&self, expected_num_nodes: usize) -> Result<()> {
        let now = SystemTime::now();
        let time_since_epoch = now.duration_since(UNIX_EPOCH)
            .context("Time went backwards")?;
        let micros_since_epoch = time_since_epoch.as_micros();
        let keygen_bin_filepath = ValidatorService::get_solana_bin_filepath(SOLANA_KEYGEN_BIN_FILENAME);
        let gossip_bin_filepath = ValidatorService::get_solana_bin_filepath(SOLANA_GOSSIP_BIN_FILENAME);
        let keypair_filepath = format!("/tmp/client-id.json-{}", micros_since_epoch);
        let cmd_args: Vec<String> = vec![
            keygen_bin_filepath,
            String::from("new"),
            String::from("--no-passphrase"),
            String::from("-fso"),
            keypair_filepath.clone(),
            String::from("&&"),
            gossip_bin_filepath,
            String::from("spy"),
            String::from("-n"),
            format!("{}:{}", self.service_context.get_ip_address(), GOSSIP_PORT),
            // NOTE: The solana-gossip CLI should normally automatically pick a free port for its gossip-port,
            // but as of 2021-03-04 there's a bug where this doesn't work so we need to specify it
            // See: https://discord.com/channels/428295358100013066/810947427544203264/817154946658992149
            String::from("--gossip-port"),
            format!("{}", GOSSIP_CLI_GOSSIP_PORT),
            String::from("--num-nodes-exactly"),
            format!("{}", expected_num_nodes),
            String::from("&&"),
            String::from("rm"),
            String::from("-rf"),
            keypair_filepath.clone(),
        ];
        let command: Vec<String> = vec![
            String::from("sh"),
            String::from("-c"),
            cmd_args.join(" "),
        ];
        debug!("Command to exec: {:?}", command);
        // REALLY annoying that we have to clone the service_context to use it, but there's no way around it - the underlying
        // Prost-generated gRPC client requires mutability
        let (exit_code, _) = self.service_context.clone().exec_command(command.clone())
            .context(format!("An error occurred executing command to assert number of nodes '{:?}'", command))?;
        
        if exit_code != SUCCESSFUL_EXIT_CODE {
            return Err(anyhow!(
                "Expected successful exit code '{}' when executing command '{:?}' but got '{}'",
                SUCCESSFUL_EXIT_CODE,
                command,
                exit_code,
            ));
        }
        return Ok(());
    }

    pub fn get_confirmed_slot(&self) -> Result<u64> {
        let params = json!([
            {
                COMMITMENT_LEVEL_PARAM: CONFIRMED_COMMMITMENT_LEVEL,
            },
        ]);
        let result = self.send(RpcRequest::GetSlot, params)
            .context("An error occurred getting the confirmed slot")?;
        return Ok(result);
    }

    pub fn dump_leader_schedule_to_file(&self) -> Result<()> {
        let ip_addr = self.service_context.get_ip_address();
        let cmd_args = vec![
            ValidatorService::get_solana_bin_filepath(SOLANA_KEYGEN_BIN_FILENAME),
            String::from("leader-schedule"),
            String::from("-u"),
            format!("http://{}:{}", ip_addr, GOSSIP_PORT),
            String::from("2>&1"),
            String::from(">"),
            format!("{}/leader-schedule-per-{}.txt", TEST_VOLUME_MOUNTPOINT, ip_addr),
        ];
        let command: Vec<String> = vec![
            String::from("sh"),
            String::from("-c"),
            cmd_args.join(" "),
        ];
        debug!("Command to exec: {:?}", command);
        // REALLY annoying that we have to clone the service_context to use it, but there's no way around it - the underlying
        // Prost-generated gRPC client requires mutability
        let (exit_code, _) = self.service_context.clone().exec_command(command.clone())
            .context(format!("An error occurred executing command to dump leader schedule to file '{:?}'", command))?;
        
        if exit_code != SUCCESSFUL_EXIT_CODE {
            return Err(anyhow!(
                "Expected successful exit code '{}' when executing command '{:?}' but got '{}'",
                SUCCESSFUL_EXIT_CODE,
                command,
                exit_code,
            ));
        }
        return Ok(());
    }

    fn send<T>(&self, request: RpcRequest, params: Value) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        assert!(params.is_array() || params.is_null());
        let response = self
            .sender
            .send(request, params)
            .context("An error occurred sending the request")?;
        let deserialized = serde_json::from_value(response)
            .context("An error occurred deserializing the response string to a JSON object")?;
        return Ok(deserialized);
    }

    fn get_solana_bin_filepath(bin_filename: &str) -> String {
        return format!("{}/{}", SOLANA_BINARIES_DIRPATH, bin_filename);
    }
}

impl Service for ValidatorService {
    fn is_available(&self) -> bool {
        let command: Vec<String> = vec![
            String::from("["),
            String::from("-r"),
            String::from(INIT_COMPLETE_FILEPATH),
            String::from("]"),
        ];
        let exec_resp_or_err = self.service_context.clone().exec_command(command);
        let exit_code: i32;
        match exec_resp_or_err {
            Ok((inner_exit_code, _)) => exit_code = inner_exit_code,
            Err(err) => {
                debug!("An error occurred executing the command to test if the init file exists: {}", err);
                return false;
            }
        }
        if exit_code != SUCCESSFUL_EXIT_CODE {
            debug!(
                "Expected successful exit code '{}' when checking if init file exists, but got '{}'",
                SUCCESSFUL_EXIT_CODE,
                exit_code,
            );
            return false;
        }
        return true;
    }
}