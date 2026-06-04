use std::io::{self, Read};

use randdb_core::{execute_request, CoreRequest, CoreResult};

fn main() {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        print_error("read_stdin", error.to_string());
        return;
    }

    let request = match serde_json::from_str::<CoreRequest>(&input) {
        Ok(request) => request,
        Err(error) => {
            print_error("parse_request", error.to_string());
            return;
        }
    };

    let result = execute_request(request);
    match serde_json::to_string_pretty(&result) {
        Ok(json) => println!("{json}"),
        Err(error) => print_error("serialize_response", error.to_string()),
    }
}

fn print_error(code: &str, message: String) {
    let result = CoreResult::Error {
        error: randdb_core::CoreError {
            code: code.to_string(),
            message,
            operation: None,
            retryable: false,
        },
    };

    match serde_json::to_string_pretty(&result) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("{error}"),
    }
}
