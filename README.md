# bank-engine

A simple toy bank (payments) engine that deals with deposits/withdrawals/disputes/resolves/chargebacks.

### Assumptions

* CSV input file is comma-delimited with headers.

* Only deposit or withdrawal can create client account.

* Disputes will only work for deposits.

* A transaction can be disputed/resolved many times, but charged back only once.

* If account is locked all operations are blocked.

### Design choices

* Bank struct expose public functions for processing transactions and writing output. All types of errors are caught. At the moment, they are simply logged in, but they can be used in a more specific way.
    * NOTE: If performance matters and there are a lot of errors, eprintln! should be removed (it locks and unlocks the stdout at each call).

* Client account keeps only deposited transactions in order to save memory.

* Using unordered data structure (hashmap) for client accounts.
    
* Used standard floats for input. Output numbers have four places past the decimal (ex. 0.0000, 1.2345).
    * In case of problems with precision, decimal can be represented with u64 / u128 (something similar to https://github.com/CosmWasm/cosmwasm/blob/main/packages/std/src/math/decimal.rs)

* A threaded design was not implemented, because the records were ordered by transaction id (processing them concurrently could lead to situation where one transaction is processed before another).
If the transaction processing operations were more complicated or blocking e.g. by reading / writing the database or sending messages to the message broker (Kafka) one could have several processing
threads which would be responsible for the relevant clients. Records would be read chronologically but processing would take place in parallel. It is important that one thread serves one client to guarantee the order of transactions.

