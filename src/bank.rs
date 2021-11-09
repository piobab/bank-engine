use csv::WriterBuilder;
use serde::Deserialize;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::HashMap;
use std::io;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum BankError {
    #[error("No account for client, id: {0}")]
    NoClientAccount(ClientId),

    #[error("Account is locked")]
    AccountIsLocked,

    #[error("Negative amount for transaction, id: {0}")]
    NegativeAmount(TxId),

    #[error("Insufficient available funds after processing transaction, id: {0}")]
    InsufficientAvailableFunds(TxId),

    #[error("No deposit transaction, id: {0}")]
    NoDepositTransaction(TxId),

    #[error("Transaction is not disputed, id: {0}")]
    TransactionIsNotDisputed(TxId),

    #[error("Transaction is already disputed, id: {0}")]
    TransactionAlreadyDisputed(TxId),
}

type ClientId = u16;
type TxId = u32;
type Balance = f32;

#[derive(Debug, Deserialize)]
pub struct Transaction {
    r#type: TxType,
    client: ClientId,
    tx: TxId,
    amount: Option<Balance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TxType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

#[derive(Default)]
pub struct Bank {
    accounts: HashMap<ClientId, Account>,
}

impl Bank {
    /// Process single transaction for client.
    pub fn process(&mut self, transaction: Transaction) -> Result<(), BankError> {
        let account = match self.accounts.entry(transaction.client) {
            Vacant(entry)
                if matches!(transaction.r#type, TxType::Deposit)
                    || matches!(transaction.r#type, TxType::Withdrawal) =>
            {
                // only deposit or withdraw can create an account
                entry.insert(Account::default())
            }
            Vacant(_) => {
                return Err(BankError::NoClientAccount(transaction.client));
            }
            Occupied(entry) => entry.into_mut(),
        };

        match transaction.r#type {
            TxType::Deposit => {
                account.deposit(transaction.tx, transaction.amount.unwrap_or_default())
            }
            TxType::Withdrawal => {
                account.withdraw(transaction.tx, transaction.amount.unwrap_or_default())
            }
            TxType::Dispute => account.dispute(transaction.tx),
            TxType::Resolve => account.resolve(transaction.tx),
            TxType::Chargeback => account.chargeback(transaction.tx),
        }
    }

    /// Write accounts to std out.
    pub fn write_accounts(&self) {
        let mut wtr = WriterBuilder::new().from_writer(io::stdout());
        let writing_headers = wtr.write_record(&["client", "available", "held", "total", "locked"]);
        if writing_headers.is_err() {
            eprintln!("Can't write account headers!");
        }
        for (client_id, account) in self.accounts.iter() {
            let writing_result = wtr.write_record(&[
                client_id.to_string(),
                format!("{:.4}", account.available),
                format!("{:.4}", account.held),
                format!("{:.4}", account.total),
                account.locked.to_string(),
            ]);
            if writing_result.is_err() {
                eprintln!(
                    "Error occurred when writing account details for client, id: {}.",
                    client_id
                );
            }
        }
    }
}

#[derive(Default)]
struct Account {
    /// The total funds that are available for trading, staking, withdrawal, etc. This
    /// should be equal to the total - held amounts.
    available: Balance,

    /// The total funds that are held for dispute. This should be equal to total -
    /// available amounts.
    held: Balance,

    /// The total funds that are available or held. This should be equal to available +
    /// held.
    total: Balance,

    /// Whether the account is locked. An account is locked if a charge back occurs.
    locked: bool,

    /// Keeps client deposit transactions. It is used if we need to dispute some deposit transaction.
    deposits: HashMap<TxId, Deposit>,
}

#[derive(Default)]
struct Deposit {
    /// Deposited amount.
    amount: Balance,

    /// Marks if dispute transaction occurred.
    disputed: bool,
}

impl Account {
    /// A deposit is a credit to the client's asset account, meaning it should increase the available and
    /// total funds of the client account.
    fn deposit(&mut self, tx_id: TxId, amount: Balance) -> Result<(), BankError> {
        self.is_locked()?;
        if amount.is_sign_negative() {
            return Err(BankError::NegativeAmount(tx_id));
        }
        self.available += amount;
        self.total += amount;
        self.deposits.insert(
            tx_id,
            Deposit {
                amount,
                disputed: false,
            },
        );
        Ok(())
    }

    /// A withdraw is a debit to the client's asset account, meaning it should decrease the available and
    /// total funds of the client account.
    fn withdraw(&mut self, tx_id: TxId, amount: Balance) -> Result<(), BankError> {
        self.is_locked()?;
        if amount.is_sign_negative() {
            return Err(BankError::NegativeAmount(tx_id));
        }
        if self.available >= amount {
            self.available -= amount;
            self.total -= amount;
            Ok(())
        } else {
            Err(BankError::InsufficientAvailableFunds(tx_id))
        }
    }

    /// A dispute represents a client's claim that a transaction was erroneous and should be reversed.
    fn dispute(&mut self, tx_id: TxId) -> Result<(), BankError> {
        self.is_locked()?;
        // `dispute` transaction doesn't have amount value. We need to find corresponding `deposit`
        // transaction
        match self.deposits.get_mut(&tx_id) {
            Some(deposit) if deposit.disputed => Err(BankError::TransactionAlreadyDisputed(tx_id)),
            Some(deposit) if self.available >= deposit.amount => {
                self.available -= deposit.amount;
                self.held += deposit.amount;
                deposit.disputed = true;
                Ok(())
            }
            Some(_) => Err(BankError::InsufficientAvailableFunds(tx_id)),
            None => Err(BankError::NoDepositTransaction(tx_id)),
        }
    }

    /// A resolve represents a resolution to a dispute, releasing the associated held funds.
    fn resolve(&mut self, tx_id: TxId) -> Result<(), BankError> {
        self.is_locked()?;
        // `resolve` transaction doesn't have amount value. We need to find corresponding `deposit`
        // transaction
        match self.deposits.get_mut(&tx_id) {
            Some(deposit) if deposit.disputed => {
                self.available += deposit.amount;
                self.held -= deposit.amount; // shouldn't be less than zero because of logic in dispute
                deposit.disputed = false;
                Ok(())
            }
            Some(_) => Err(BankError::TransactionIsNotDisputed(tx_id)),
            None => Err(BankError::NoDepositTransaction(tx_id)),
        }
    }

    /// A chargeback is the final state of a dispute and represents the client reversing a transaction.
    fn chargeback(&mut self, tx_id: TxId) -> Result<(), BankError> {
        self.is_locked()?;
        // `chargeback` transaction doesn't have amount value. We need to find corresponding `deposit`
        // transaction
        match self.deposits.get_mut(&tx_id) {
            Some(deposit) if deposit.disputed => {
                // shouldn't be less than zero because of logic in dispute
                self.held -= deposit.amount;
                self.total -= deposit.amount;
                deposit.disputed = false;
                self.locked = true;
                Ok(())
            }
            Some(_) => Err(BankError::TransactionIsNotDisputed(tx_id)),
            None => Err(BankError::NoDepositTransaction(tx_id)),
        }
    }

    /// Verify if account is locked.
    fn is_locked(&self) -> Result<(), BankError> {
        if self.locked {
            return Err(BankError::AccountIsLocked);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposit() {
        let mut account = Account::default();

        account.deposit(1, 120.55).unwrap();
        account.deposit(3, 130.66).unwrap();

        assert_eq!(account.available, 120.55 + 130.66);
        assert_eq!(account.total, 120.55 + 130.66);
        assert_eq!(account.held, 0.0);
        assert!(!account.locked);
        assert!(account.deposits.contains_key(&1));
        assert!(!account.deposits.contains_key(&2));
        assert!(account.deposits.contains_key(&3));
    }

    #[test]
    fn test_deposit_if_amount_negative() {
        let mut account = Account::default();
        let error_res = account.deposit(1, -120.55).unwrap_err();
        assert_eq!(error_res, BankError::NegativeAmount(1));
    }

    #[test]
    fn test_withdraw() {
        let mut account = Account::default();
        account.available = 200.0;
        account.total = 200.0;

        account.withdraw(2, 128.68).unwrap();

        assert_eq!(account.available, 200.0 - 128.68);
        assert_eq!(account.total, 200.0 - 128.68);
        assert_eq!(account.held, 0.0);
        assert!(!account.locked);
    }

    #[test]
    fn test_withdraw_if_amount_negative() {
        let mut account = Account::default();
        let error_res = account.withdraw(3, -120.55).unwrap_err();
        assert_eq!(error_res, BankError::NegativeAmount(3));
    }

    #[test]
    fn test_withdraw_if_insufficient_funds() {
        let mut account = Account::default();
        account.available = 200.0;
        account.total = 200.0;

        let error_res = account.withdraw(10, 200.10).unwrap_err();

        assert_eq!(error_res, BankError::InsufficientAvailableFunds(10));
    }

    #[test]
    fn test_dispute() {
        let mut account = Account::default();
        account.available = 200.0;
        account.total = 200.0;
        account.deposits.insert(
            111,
            Deposit {
                amount: 100.0,
                disputed: false,
            },
        );

        account.dispute(111).unwrap();

        assert_eq!(account.available, 200.0 - 100.0);
        assert_eq!(account.total, 200.0);
        assert_eq!(account.held, 100.0);
        assert!(!account.locked);
        assert!(account.deposits.get(&111).unwrap().disputed);
    }

    #[test]
    fn test_double_dispute() {
        let mut account = Account::default();
        account.available = 350.0;
        account.total = 350.0;
        account.deposits.insert(
            112,
            Deposit {
                amount: 100.0,
                disputed: false,
            },
        );

        account.dispute(112).unwrap();
        let error_res = account.dispute(112).unwrap_err();
        assert_eq!(error_res, BankError::TransactionAlreadyDisputed(112));
    }

    #[test]
    fn test_dispute_if_no_deposit_transaction() {
        let mut account = Account::default();
        let error_res = account.dispute(112).unwrap_err();
        assert_eq!(error_res, BankError::NoDepositTransaction(112));
    }

    #[test]
    fn test_dispute_if_insufficient_funds() {
        let mut account = Account::default();
        account.available = 200.0;
        account.total = 200.0;
        account.deposits.insert(
            1,
            Deposit {
                amount: 201.0,
                disputed: false,
            },
        );

        let error_res = account.dispute(1).unwrap_err();

        assert_eq!(error_res, BankError::InsufficientAvailableFunds(1));
    }

    #[test]
    fn test_resolve() {
        let mut account = Account::default();
        account.available = 150.0;
        account.total = 200.0;
        account.held = 50.0;
        account.deposits.insert(
            10,
            Deposit {
                amount: 50.0,
                disputed: true,
            },
        );

        account.resolve(10).unwrap();

        assert_eq!(account.available, 200.0);
        assert_eq!(account.total, 200.0);
        assert_eq!(account.held, 0.0);
        assert!(!account.locked);
        assert!(!account.deposits.get(&10).unwrap().disputed);
    }

    #[test]
    fn test_resolve_if_no_deposit_transaction() {
        let mut account = Account::default();
        let error_res = account.resolve(112).unwrap_err();
        assert_eq!(error_res, BankError::NoDepositTransaction(112));
    }

    #[test]
    fn test_resolve_if_transaction_is_not_disputed() {
        let mut account = Account::default();
        account.available = 150.0;
        account.total = 150.0;
        account.held = 0.0;
        account.deposits.insert(
            10,
            Deposit {
                amount: 150.0,
                disputed: false,
            },
        );

        let error_res = account.resolve(10).unwrap_err();

        assert_eq!(error_res, BankError::TransactionIsNotDisputed(10));
    }

    #[test]
    fn test_chargeback() {
        let mut account = Account::default();
        account.available = 150.0;
        account.total = 200.0;
        account.held = 50.0;
        account.deposits.insert(
            10,
            Deposit {
                amount: 50.0,
                disputed: true,
            },
        );

        account.chargeback(10).unwrap();

        assert_eq!(account.available, 150.0);
        assert_eq!(account.total, 150.0);
        assert_eq!(account.held, 0.0);
        assert!(account.locked);
        assert!(!account.deposits.get(&10).unwrap().disputed);
    }

    #[test]
    fn test_chargeback_if_no_deposit_transaction() {
        let mut account = Account::default();
        let error_res = account.chargeback(112).unwrap_err();
        assert_eq!(error_res, BankError::NoDepositTransaction(112));
    }

    #[test]
    fn test_chargeback_if_transaction_is_not_disputed() {
        let mut account = Account::default();
        account.available = 160.0;
        account.total = 160.0;
        account.held = 0.0;
        account.deposits.insert(
            100,
            Deposit {
                amount: 160.0,
                disputed: false,
            },
        );

        let error_res = account.chargeback(100).unwrap_err();

        assert_eq!(error_res, BankError::TransactionIsNotDisputed(100));
    }

    #[test]
    fn test_locked_account() {
        let mut account = Account::default();
        account.locked = true;

        let error_res = account.deposit(1, 10.0).unwrap_err();
        assert_eq!(error_res, BankError::AccountIsLocked);
        let error_res = account.withdraw(2, 8.15).unwrap_err();
        assert_eq!(error_res, BankError::AccountIsLocked);
        let error_res = account.dispute(1).unwrap_err();
        assert_eq!(error_res, BankError::AccountIsLocked);
        let error_res = account.resolve(1).unwrap_err();
        assert_eq!(error_res, BankError::AccountIsLocked);
        let error_res = account.chargeback(1).unwrap_err();
        assert_eq!(error_res, BankError::AccountIsLocked);
    }
}
