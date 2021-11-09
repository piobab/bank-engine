use crate::bank::Bank;
use csv::{ReaderBuilder, Trim};
use std::time::Instant;

mod bank;

fn main() {
    let input_path = std::env::args().nth(1).expect("No input file!");

    let now = Instant::now();

    // NOTE: I consciously panic in case of critical errors. The rest is logged.
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .delimiter(b',')
        .trim(Trim::All)
        .from_path(input_path)
        .expect("Can't create csv reader!");

    let mut bank = Bank::default();
    for record in rdr.deserialize() {
        match record {
            Ok(transaction) => {
                let processing_result = bank.process(transaction);
                if let Err(error) = processing_result {
                    eprintln!("Error occurred when processing transaction. {}.", error);
                }
            }
            Err(error) => {
                eprintln!("Can't deserialize transaction. Error: {:?}.", error);
            }
        }
    }
    bank.write_accounts();

    eprintln!("Processed in: {} millis", now.elapsed().as_millis());
}
