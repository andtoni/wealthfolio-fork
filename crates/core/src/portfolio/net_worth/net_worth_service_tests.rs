//! Unit tests for net worth service.

#![allow(clippy::type_complexity)]

use super::*;
use crate::accounts::{
    account_types, Account, AccountRepositoryTrait, AccountUpdate,
    AggregateEligibilityServiceTrait, NewAccount, ProvenanceRole, VerifiedReceipt,
    VerifiedReceiptRepositoryTrait, ENTITY_DAILY_VALUATION, ENTITY_SNAPSHOT,
};
use crate::assets::{
    Asset, AssetKind, AssetRepositoryTrait, InstrumentType, NewAsset, ProviderProfile, QuoteMode,
    UpdateAssetProfile,
};
use crate::errors::Result;
use crate::fx::{ExchangeRate, FxServiceTrait, NewExchangeRate};
use crate::portfolio::economic_events::BasisStatus;
use crate::portfolio::snapshot::{
    AccountStateSnapshot, Position, SnapshotRepositoryTrait, SnapshotSource,
};
use crate::portfolio::valuation::{
    DailyAccountValuation, ExternalFlowSource, NegativeBalanceInfo, ValuationRepositoryTrait,
    ValuationStatus,
};
use crate::quotes::{
    LatestQuotePair, LatestQuoteSnapshot, ProviderInfo, Quote, QuoteImport, QuoteServiceTrait,
    QuoteSyncState, SymbolSearchResult, SymbolSyncPlan, SyncResult,
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

// ============================================================================
// Mock Implementations
// ============================================================================

struct MockAccountRepository {
    accounts: Vec<Account>,
}

impl MockAccountRepository {
    fn new(accounts: Vec<Account>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl AccountRepositoryTrait for MockAccountRepository {
    async fn create(&self, _new_account: NewAccount) -> Result<Account> {
        unimplemented!()
    }

    async fn update(&self, _account_update: AccountUpdate) -> Result<Account> {
        unimplemented!()
    }

    async fn delete(&self, _account_id: &str) -> Result<usize> {
        unimplemented!()
    }

    fn get_by_id(&self, account_id: &str) -> Result<Account> {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .cloned()
            .ok_or_else(|| {
                crate::errors::Error::Repository(format!("Account {} not found", account_id))
            })
    }

    fn list(
        &self,
        is_active_filter: Option<bool>,
        is_archived_filter: Option<bool>,
        _account_ids: Option<&[String]>,
    ) -> Result<Vec<Account>> {
        let accounts: Vec<Account> = self
            .accounts
            .iter()
            .filter(|a| is_active_filter.is_none_or(|active| a.is_active == active))
            .filter(|a| is_archived_filter.is_none_or(|archived| a.is_archived == archived))
            .cloned()
            .collect();
        Ok(accounts)
    }
}

struct MockAssetRepository {
    assets: Vec<Asset>,
}

impl MockAssetRepository {
    fn new(assets: Vec<Asset>) -> Self {
        Self { assets }
    }
}

#[async_trait]
impl AssetRepositoryTrait for MockAssetRepository {
    async fn create(&self, _new_asset: NewAsset) -> Result<Asset> {
        unimplemented!()
    }

    async fn create_batch(&self, _new_assets: Vec<NewAsset>) -> Result<Vec<Asset>> {
        unimplemented!()
    }

    async fn update_profile(&self, _asset_id: &str, _payload: UpdateAssetProfile) -> Result<Asset> {
        unimplemented!()
    }

    async fn update_quote_mode(&self, _asset_id: &str, _quote_mode: &str) -> Result<Asset> {
        unimplemented!()
    }

    fn find_by_instrument_key(&self, _instrument_key: &str) -> Result<Option<Asset>> {
        Ok(None)
    }

    fn get_by_id(&self, asset_id: &str) -> Result<Asset> {
        self.assets
            .iter()
            .find(|a| a.id == asset_id)
            .cloned()
            .ok_or_else(|| {
                crate::errors::Error::Repository(format!("Asset {} not found", asset_id))
            })
    }

    fn list(&self) -> Result<Vec<Asset>> {
        Ok(self.assets.clone())
    }

    fn list_by_asset_ids(&self, asset_ids: &[String]) -> Result<Vec<Asset>> {
        Ok(self
            .assets
            .iter()
            .filter(|a| asset_ids.contains(&a.id))
            .cloned()
            .collect())
    }

    async fn delete(&self, _asset_id: &str) -> Result<()> {
        unimplemented!()
    }

    fn search_by_symbol(&self, _query: &str) -> Result<Vec<Asset>> {
        Ok(Vec::new())
    }

    async fn cleanup_legacy_metadata(&self, _asset_id: &str) -> Result<()> {
        Ok(())
    }

    async fn deactivate(&self, _asset_id: &str) -> Result<()> {
        Ok(())
    }

    async fn reactivate(&self, _asset_id: &str) -> Result<()> {
        Ok(())
    }

    async fn copy_user_metadata(&self, _source_id: &str, _target_id: &str) -> Result<()> {
        Ok(())
    }

    async fn deactivate_orphaned_investments(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

struct MockSnapshotRepository {
    snapshots: HashMap<String, AccountStateSnapshot>,
}

impl MockSnapshotRepository {
    fn new(snapshots: Vec<AccountStateSnapshot>) -> Self {
        let map = snapshots
            .into_iter()
            .map(|s| (s.account_id.clone(), s))
            .collect();
        Self { snapshots: map }
    }
}

#[async_trait]
impl SnapshotRepositoryTrait for MockSnapshotRepository {
    async fn save_snapshots(&self, _snapshots: &[AccountStateSnapshot]) -> Result<()> {
        unimplemented!()
    }

    fn get_snapshots_by_account(
        &self,
        _account_id: &str,
        _start_date: Option<NaiveDate>,
        _end_date: Option<NaiveDate>,
    ) -> Result<Vec<AccountStateSnapshot>> {
        unimplemented!()
    }

    fn get_latest_snapshot_before_date(
        &self,
        account_id: &str,
        _date: NaiveDate,
    ) -> Result<Option<AccountStateSnapshot>> {
        Ok(self.snapshots.get(account_id).cloned())
    }

    fn get_latest_snapshots_before_date(
        &self,
        account_ids: &[String],
        _date: NaiveDate,
    ) -> Result<HashMap<String, AccountStateSnapshot>> {
        let result = account_ids
            .iter()
            .filter_map(|id| self.snapshots.get(id).map(|s| (id.clone(), s.clone())))
            .collect();
        Ok(result)
    }

    fn get_all_latest_snapshots(
        &self,
        account_ids: &[String],
    ) -> Result<HashMap<String, AccountStateSnapshot>> {
        let result = account_ids
            .iter()
            .filter_map(|id| self.snapshots.get(id).map(|s| (id.clone(), s.clone())))
            .collect();
        Ok(result)
    }

    async fn delete_snapshots_by_account_ids(&self, _account_ids: &[String]) -> Result<usize> {
        unimplemented!()
    }

    async fn delete_snapshots_for_account_and_dates(
        &self,
        _account_id: &str,
        _dates_to_delete: &[NaiveDate],
    ) -> Result<()> {
        unimplemented!()
    }

    async fn delete_snapshots_for_account_in_range(
        &self,
        _account_id: &str,
        _start_date: NaiveDate,
        _end_date: NaiveDate,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn overwrite_snapshots_for_account_in_range(
        &self,
        _account_id: &str,
        _start_date: NaiveDate,
        _end_date: NaiveDate,
        _snapshots_to_save: &[AccountStateSnapshot],
    ) -> Result<()> {
        unimplemented!()
    }

    async fn overwrite_multiple_account_snapshot_ranges(
        &self,
        _new_snapshots: &[AccountStateSnapshot],
    ) -> Result<()> {
        unimplemented!()
    }

    fn get_all_non_archived_account_snapshots(
        &self,
        _start_date: Option<NaiveDate>,
        _end_date: Option<NaiveDate>,
    ) -> Result<Vec<AccountStateSnapshot>> {
        unimplemented!()
    }

    fn get_earliest_snapshot_date(&self, _account_id: &str) -> Result<Option<NaiveDate>> {
        unimplemented!()
    }

    async fn overwrite_all_snapshots_for_account(
        &self,
        _account_id: &str,
        _snapshots_to_save: &[AccountStateSnapshot],
    ) -> Result<()> {
        unimplemented!()
    }

    async fn update_snapshots_source(&self, _account_id: &str, _new_source: &str) -> Result<usize> {
        unimplemented!()
    }

    async fn save_or_update_snapshot(&self, _snapshot: &AccountStateSnapshot) -> Result<()> {
        unimplemented!()
    }

    fn get_non_calculated_snapshot_count(&self, _account_id: &str) -> Result<usize> {
        Ok(0)
    }

    fn get_earliest_non_calculated_snapshot(
        &self,
        _account_id: &str,
    ) -> Result<Option<AccountStateSnapshot>> {
        Ok(None)
    }

    fn get_snapshot_positions(&self, _snapshot_id: &str) -> Result<HashMap<String, Position>> {
        Ok(HashMap::new())
    }

    fn get_snapshot_positions_batch(
        &self,
        _snapshot_ids: &[String],
    ) -> Result<HashMap<String, HashMap<String, Position>>> {
        Ok(HashMap::new())
    }
}

struct MockMarketDataRepository {
    quotes: Vec<Quote>,
}

impl MockMarketDataRepository {
    fn new(quotes: Vec<Quote>) -> Self {
        Self { quotes }
    }
}

#[async_trait]
impl QuoteServiceTrait for MockMarketDataRepository {
    // =========================================================================
    // Quote CRUD Operations
    // =========================================================================

    fn get_latest_quote(&self, symbol: &str) -> Result<Quote> {
        self.quotes
            .iter()
            .filter(|q| q.asset_id == symbol)
            .max_by_key(|q| q.timestamp)
            .cloned()
            .ok_or_else(|| {
                crate::errors::Error::Repository(format!("Quote not found for {}", symbol))
            })
    }

    fn get_latest_quotes(&self, symbols: &[String]) -> Result<HashMap<String, Quote>> {
        let mut result = HashMap::new();
        for symbol in symbols {
            if let Some(quote) = self
                .quotes
                .iter()
                .filter(|q| &q.asset_id == symbol)
                .max_by_key(|q| q.timestamp)
            {
                result.insert(symbol.clone(), quote.clone());
            }
        }
        Ok(result)
    }

    fn get_latest_quotes_as_of(
        &self,
        _symbols: &[String],
        _as_of: chrono::NaiveDate,
    ) -> Result<HashMap<String, Quote>> {
        Ok(HashMap::new())
    }

    fn get_latest_quotes_snapshot(
        &self,
        asset_ids: &[String],
    ) -> Result<HashMap<String, LatestQuoteSnapshot>> {
        let today = Utc::now().date_naive();
        let quotes = self.get_latest_quotes(asset_ids)?;
        Ok(quotes
            .into_iter()
            .map(|(asset_id, quote)| {
                let quote_day = quote.timestamp.date_naive();
                (
                    asset_id,
                    LatestQuoteSnapshot {
                        quote: Some(quote),
                        is_stale: quote_day < today,
                        effective_market_date: today.to_string(),
                        quote_date: Some(quote_day.to_string()),
                        no_quote_reason: None,
                    },
                )
            })
            .collect())
    }

    fn get_latest_quotes_pair(
        &self,
        _symbols: &[String],
    ) -> Result<HashMap<String, LatestQuotePair>> {
        unimplemented!()
    }

    fn get_historical_quotes(&self, symbol: &str) -> Result<Vec<Quote>> {
        Ok(self
            .quotes
            .iter()
            .filter(|q| q.asset_id == symbol)
            .cloned()
            .collect())
    }

    fn get_all_historical_quotes(&self) -> Result<HashMap<String, Vec<(NaiveDate, Quote)>>> {
        let mut result: HashMap<String, Vec<(NaiveDate, Quote)>> = HashMap::new();
        for quote in &self.quotes {
            let date = quote.timestamp.date_naive();
            result
                .entry(quote.asset_id.clone())
                .or_default()
                .push((date, quote.clone()));
        }
        Ok(result)
    }

    fn get_quotes_in_range(
        &self,
        symbols: &HashSet<String>,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<Quote>> {
        Ok(self
            .quotes
            .iter()
            .filter(|q| {
                let date = q.timestamp.date_naive();
                symbols.contains(&q.asset_id) && date >= start && date <= end
            })
            .cloned()
            .collect())
    }

    fn get_quotes_in_range_filled(
        &self,
        symbols: &HashSet<String>,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<Quote>> {
        // For testing, just return the raw quotes in range
        // The actual implementation would fill gaps
        self.get_quotes_in_range(symbols, start, end)
    }

    async fn get_daily_quotes(
        &self,
        _asset_ids: &HashSet<String>,
        _start: NaiveDate,
        _end: NaiveDate,
    ) -> Result<HashMap<NaiveDate, HashMap<String, Quote>>> {
        unimplemented!()
    }

    async fn add_quote(&self, _quote: &Quote) -> Result<Quote> {
        unimplemented!()
    }

    async fn update_quote(&self, _quote: Quote) -> Result<Quote> {
        unimplemented!()
    }

    async fn delete_quote(&self, _quote_id: &str) -> Result<()> {
        unimplemented!()
    }

    async fn bulk_upsert_quotes(&self, _quotes: Vec<Quote>) -> Result<usize> {
        unimplemented!()
    }

    // =========================================================================
    // Provider Operations
    // =========================================================================

    async fn search_symbol(&self, _query: &str) -> Result<Vec<SymbolSearchResult>> {
        unimplemented!()
    }

    async fn search_symbol_with_currency(
        &self,
        _query: &str,
        _account_currency: Option<&str>,
    ) -> Result<Vec<SymbolSearchResult>> {
        unimplemented!()
    }

    async fn get_asset_profile(&self, _asset: &Asset) -> Result<ProviderProfile> {
        unimplemented!()
    }

    async fn fetch_quotes_from_provider(
        &self,
        _asset_id: &str,
        _start: NaiveDate,
        _end: NaiveDate,
    ) -> Result<Vec<Quote>> {
        unimplemented!()
    }

    async fn fetch_quotes_for_symbol(
        &self,
        _symbol: &str,
        _currency: &str,
        _start: NaiveDate,
        _end: NaiveDate,
    ) -> Result<Vec<Quote>> {
        unimplemented!()
    }

    // =========================================================================
    // Sync Operations
    // =========================================================================

    async fn sync(
        &self,
        _mode: crate::quotes::SyncMode,
        _asset_ids: Option<Vec<String>>,
    ) -> Result<SyncResult> {
        unimplemented!()
    }

    async fn resync(&self, _asset_ids: Option<Vec<String>>) -> Result<SyncResult> {
        unimplemented!()
    }

    async fn refresh_sync_state(&self) -> Result<()> {
        Ok(())
    }

    fn get_sync_plan(&self) -> Result<Vec<SymbolSyncPlan>> {
        Ok(Vec::new())
    }

    async fn handle_activity_created(
        &self,
        _symbol: &str,
        _activity_date: NaiveDate,
    ) -> Result<()> {
        Ok(())
    }

    async fn handle_activity_deleted(&self, _symbol: &str) -> Result<()> {
        Ok(())
    }

    async fn delete_sync_state(&self, _symbol: &str) -> Result<()> {
        Ok(())
    }

    fn get_symbols_needing_sync(&self) -> Result<Vec<QuoteSyncState>> {
        Ok(Vec::new())
    }

    fn get_sync_state(&self, _symbol: &str) -> Result<Option<QuoteSyncState>> {
        Ok(None)
    }

    async fn mark_profile_enriched(&self, _symbol: &str) -> Result<()> {
        Ok(())
    }

    fn get_assets_needing_profile_enrichment(&self) -> Result<Vec<QuoteSyncState>> {
        Ok(Vec::new())
    }

    async fn update_position_status_from_holdings(
        &self,
        _current_holdings: &std::collections::HashMap<String, rust_decimal::Decimal>,
    ) -> Result<()> {
        Ok(())
    }

    fn get_sync_states_with_errors(&self) -> Result<Vec<QuoteSyncState>> {
        Ok(Vec::new())
    }

    async fn reset_sync_errors(&self, _asset_ids: &[String]) -> Result<()> {
        Ok(())
    }

    async fn reset_sync_state_for_profile_change(&self, _asset_id: &str) -> Result<()> {
        Ok(())
    }

    // =========================================================================
    // Provider Settings
    // =========================================================================

    async fn get_providers_info(&self) -> Result<Vec<ProviderInfo>> {
        unimplemented!()
    }

    async fn update_provider_settings(
        &self,
        _provider_id: &str,
        _priority: i32,
        _enabled: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    // =========================================================================
    // Quote Import
    // =========================================================================

    async fn check_quotes_import(
        &self,
        _content: &[u8],
        _has_header_row: bool,
    ) -> Result<Vec<QuoteImport>> {
        unimplemented!()
    }

    async fn import_quotes(
        &self,
        _quotes: Vec<QuoteImport>,
        _overwrite: bool,
    ) -> Result<Vec<QuoteImport>> {
        unimplemented!()
    }
}

struct MockFxService {
    #[allow(dead_code)]
    base_currency: String,
}

impl MockFxService {
    fn new(base_currency: &str) -> Self {
        Self {
            base_currency: base_currency.to_string(),
        }
    }
}

#[async_trait]
impl FxServiceTrait for MockFxService {
    fn initialize(&self) -> Result<()> {
        Ok(())
    }

    fn get_historical_rates(
        &self,
        _from_currency: &str,
        _to_currency: &str,
        _days: i64,
    ) -> Result<Vec<ExchangeRate>> {
        Ok(vec![])
    }

    fn get_latest_exchange_rate(
        &self,
        _from_currency: &str,
        _to_currency: &str,
    ) -> Result<Decimal> {
        Ok(dec!(1.0))
    }

    fn get_exchange_rate_for_date(
        &self,
        _from_currency: &str,
        _to_currency: &str,
        _date: NaiveDate,
    ) -> Result<Decimal> {
        Ok(dec!(1.0))
    }

    fn convert_currency(
        &self,
        amount: Decimal,
        from_currency: &str,
        to_currency: &str,
    ) -> Result<Decimal> {
        if from_currency == to_currency {
            return Ok(amount);
        }
        // For testing, use 1:1 conversion
        Ok(amount)
    }

    fn convert_currency_for_date(
        &self,
        amount: Decimal,
        from_currency: &str,
        to_currency: &str,
        _date: NaiveDate,
    ) -> Result<Decimal> {
        if from_currency == to_currency {
            return Ok(amount);
        }
        // For testing, use 1:1 conversion
        Ok(amount)
    }

    fn get_latest_exchange_rates(&self) -> Result<Vec<ExchangeRate>> {
        Ok(vec![])
    }

    async fn add_exchange_rate(&self, _new_rate: NewExchangeRate) -> Result<ExchangeRate> {
        unimplemented!()
    }

    async fn update_exchange_rate(
        &self,
        _from_currency: &str,
        _to_currency: &str,
        _rate: Decimal,
    ) -> Result<ExchangeRate> {
        unimplemented!()
    }

    async fn delete_exchange_rate(&self, _rate_id: &str) -> Result<()> {
        unimplemented!()
    }

    async fn register_currency_pair(&self, _from_currency: &str, _to_currency: &str) -> Result<()> {
        Ok(())
    }

    async fn register_currency_pair_manual(
        &self,
        _from_currency: &str,
        _to_currency: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn ensure_fx_pairs(&self, _pairs: Vec<(String, String)>) -> Result<()> {
        Ok(())
    }
}

struct MockValuationRepository {
    valuations: Vec<DailyAccountValuation>,
}

impl MockValuationRepository {
    fn new(valuations: Vec<DailyAccountValuation>) -> Self {
        Self { valuations }
    }
}

#[async_trait]
impl ValuationRepositoryTrait for MockValuationRepository {
    async fn save_valuations(&self, _valuation_records: &[DailyAccountValuation]) -> Result<()> {
        Ok(())
    }

    async fn replace_valuations_for_account(
        &self,
        _account_id: &str,
        _since_date: Option<NaiveDate>,
        _valuation_records: &[DailyAccountValuation],
    ) -> Result<()> {
        Ok(())
    }

    fn get_historical_valuations(
        &self,
        account_id: &str,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<DailyAccountValuation>> {
        let filtered: Vec<_> = self
            .valuations
            .iter()
            .filter(|v| v.account_id == account_id)
            .filter(|v| start_date.map(|sd| v.valuation_date >= sd).unwrap_or(true))
            .filter(|v| end_date.map(|ed| v.valuation_date <= ed).unwrap_or(true))
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn get_historical_valuations_for_accounts(
        &self,
        account_ids: &[String],
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<DailyAccountValuation>> {
        let filtered: Vec<_> = self
            .valuations
            .iter()
            .filter(|v| account_ids.contains(&v.account_id))
            .filter(|v| start_date.map(|sd| v.valuation_date >= sd).unwrap_or(true))
            .filter(|v| end_date.map(|ed| v.valuation_date <= ed).unwrap_or(true))
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn load_latest_valuation_date(&self, account_id: &str) -> Result<Option<NaiveDate>> {
        let latest = self
            .valuations
            .iter()
            .filter(|v| v.account_id == account_id)
            .max_by_key(|v| v.valuation_date)
            .map(|v| v.valuation_date);
        Ok(latest)
    }

    async fn delete_valuations_for_account(
        &self,
        _account_id: &str,
        _since_date: Option<NaiveDate>,
    ) -> Result<()> {
        Ok(())
    }

    fn get_latest_valuations(&self, account_ids: &[String]) -> Result<Vec<DailyAccountValuation>> {
        let mut result = vec![];
        for account_id in account_ids {
            if let Some(val) = self
                .valuations
                .iter()
                .filter(|v| &v.account_id == account_id)
                .max_by_key(|v| v.valuation_date)
            {
                result.push(val.clone());
            }
        }
        Ok(result)
    }

    fn get_valuations_on_date(
        &self,
        account_ids: &[String],
        date: NaiveDate,
    ) -> Result<Vec<DailyAccountValuation>> {
        let filtered: Vec<_> = self
            .valuations
            .iter()
            .filter(|v| account_ids.contains(&v.account_id) && v.valuation_date == date)
            .cloned()
            .collect();
        Ok(filtered)
    }

    fn get_accounts_with_negative_balance(
        &self,
        _account_ids: &[String],
    ) -> Result<Vec<NegativeBalanceInfo>> {
        Ok(Vec::new())
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_account(id: &str, account_type: &str, currency: &str) -> Account {
    let now = Utc::now().naive_utc();
    Account {
        include_in_net_worth: true,
        name_source: "provider".to_string(),
        id: id.to_string(),
        name: format!("Test Account {}", id),
        account_type: account_type.to_string(),
        group: None,
        currency: currency.to_string(),
        is_default: false,
        is_active: true,
        created_at: now,
        updated_at: now,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        is_archived: false,
        tracking_mode: crate::accounts::TrackingMode::NotSet,
    }
}

fn create_test_asset(id: &str, kind: AssetKind, currency: &str) -> Asset {
    let now = Utc::now().naive_utc();
    Asset {
        id: id.to_string(),
        name: Some(format!("Asset {}", id)),
        display_code: Some(id.to_string()),
        notes: None,
        created_at: now,
        updated_at: now,
        quote_ccy: currency.to_string(),
        kind,
        instrument_type: None,
        instrument_symbol: None,
        instrument_exchange_mic: None,
        instrument_key: None,
        exchange_name: None,
        quote_mode: QuoteMode::Market,
        provider_config: None,
        is_active: true,
        metadata: None,
    }
}

fn create_test_position(
    account_id: &str,
    asset_id: &str,
    quantity: Decimal,
    cost_basis: Decimal,
    currency: &str,
) -> Position {
    Position {
        id: format!("POS-{}-{}", asset_id, account_id),
        account_id: account_id.to_string(),
        asset_id: asset_id.to_string(),
        quantity,
        average_cost: if quantity > Decimal::ZERO {
            cost_basis / quantity
        } else {
            Decimal::ZERO
        },
        total_cost_basis: cost_basis,
        currency: currency.to_string(),
        inception_date: Utc::now(),
        lots: VecDeque::new(),
        created_at: Utc::now(),
        last_updated: Utc::now(),
        is_alternative: false,
        contract_multiplier: Decimal::ONE,
    }
}

fn create_test_snapshot(
    account_id: &str,
    positions: Vec<Position>,
    cash: HashMap<String, Decimal>,
) -> AccountStateSnapshot {
    let mut positions_map = HashMap::new();
    for pos in positions {
        positions_map.insert(pos.asset_id.clone(), pos);
    }

    AccountStateSnapshot {
        id: format!("{}_2024-01-15", account_id),
        account_id: account_id.to_string(),
        snapshot_date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        currency: "USD".to_string(),
        positions: positions_map,
        cash_balances: cash,
        cost_basis: Decimal::ZERO,
        net_contribution: Decimal::ZERO,
        net_contribution_base: Decimal::ZERO,
        cash_total_account_currency: Decimal::ZERO,
        cash_total_base_currency: Decimal::ZERO,
        position_market_values: HashMap::new(),
        liability_balance: Decimal::ZERO,
        calculated_at: Utc::now().naive_utc(),
        source: SnapshotSource::Calculated,
    }
}

fn create_test_quote(symbol: &str, price: Decimal, date: NaiveDate, currency: &str) -> Quote {
    Quote {
        id: format!("{}-{}", symbol, date),
        asset_id: symbol.to_string(),
        timestamp: DateTime::from_naive_utc_and_offset(date.and_hms_opt(16, 0, 0).unwrap(), Utc),
        open: price,
        high: price,
        low: price,
        close: price,
        adjclose: price,
        volume: dec!(0),
        currency: currency.to_string(),
        data_source: "MANUAL".to_string(),
        created_at: Utc::now(),
        notes: None,
    }
}

// ---- Mock Verified Receipt Repository ----

struct MockVerifiedReceiptRepository {
    eligible_ids: Vec<String>,
}

impl MockVerifiedReceiptRepository {
    #[allow(dead_code)]
    fn new(eligible_ids: Vec<String>) -> Self {
        Self { eligible_ids }
    }
}

#[async_trait]
impl VerifiedReceiptRepositoryTrait for MockVerifiedReceiptRepository {
    async fn get_by_account(&self, _account_id: &str) -> Result<Option<VerifiedReceipt>> {
        Ok(None)
    }

    async fn upsert(&self, _receipt: VerifiedReceipt) -> Result<VerifiedReceipt> {
        unimplemented!()
    }

    async fn delete(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    async fn get_eligible_account_ids(&self) -> Result<Vec<String>> {
        Ok(self.eligible_ids.clone())
    }
}

struct MockAggregateEligibilityService {
    eligible_account_ids: HashSet<String>,
    eligible_snapshot_ids: HashSet<String>,
    eligible_valuation_ids: HashSet<String>,
}

impl MockAggregateEligibilityService {
    fn allow_all_for(
        accounts: &[Account],
        snapshots: &[AccountStateSnapshot],
        valuations: &[DailyAccountValuation],
    ) -> Self {
        Self {
            eligible_account_ids: accounts
                .iter()
                .filter(|account| account.include_in_net_worth)
                .map(|account| account.id.clone())
                .collect(),
            eligible_snapshot_ids: snapshots
                .iter()
                .map(|snapshot| snapshot.id.clone())
                .collect(),
            eligible_valuation_ids: valuations
                .iter()
                .map(|valuation| valuation.id.clone())
                .collect(),
        }
    }

    fn with_rows(
        eligible_account_ids: &[&str],
        eligible_snapshot_ids: &[&str],
        eligible_valuation_ids: &[&str],
    ) -> Self {
        Self {
            eligible_account_ids: eligible_account_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
            eligible_snapshot_ids: eligible_snapshot_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
            eligible_valuation_ids: eligible_valuation_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
        }
    }
}

#[async_trait]
impl AggregateEligibilityServiceTrait for MockAggregateEligibilityService {
    async fn eligible_account_ids(
        &self,
        requested_account_ids: &[String],
        _as_of: NaiveDate,
    ) -> Result<HashSet<String>> {
        Ok(requested_account_ids
            .iter()
            .filter(|id| self.eligible_account_ids.contains(*id))
            .cloned()
            .collect())
    }

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>> {
        let allowed = match entity_type {
            ENTITY_SNAPSHOT => &self.eligible_snapshot_ids,
            ENTITY_DAILY_VALUATION => &self.eligible_valuation_ids,
            _ => return Ok(HashSet::new()),
        };
        Ok(rows
            .iter()
            .filter(|&(id, _)| allowed.contains(id))
            .map(|(id, _)| id.clone())
            .collect())
    }
}

#[derive(Clone)]
struct SemanticPolicyRecord {
    role: ProvenanceRole,
    coverage_start_date: Option<NaiveDate>,
    manual_current_approved: bool,
}

impl SemanticPolicyRecord {
    fn live() -> Self {
        Self {
            role: ProvenanceRole::LiveCanonical,
            coverage_start_date: None,
            manual_current_approved: false,
        }
    }

    fn historical_context() -> Self {
        Self {
            role: ProvenanceRole::HistoricalContext,
            coverage_start_date: None,
            manual_current_approved: false,
        }
    }

    fn is_eligible(&self, date: NaiveDate) -> bool {
        match self.role {
            ProvenanceRole::LiveCanonical => true,
            ProvenanceRole::OpeningBaseline => self
                .coverage_start_date
                .is_some_and(|coverage_start| date >= coverage_start),
            ProvenanceRole::ManualCurrent => self.manual_current_approved,
            ProvenanceRole::HistoricalContext | ProvenanceRole::ContextualReference => false,
        }
    }
}

#[derive(Default)]
struct RecordingSemanticEligibilityService {
    account_records: HashMap<String, SemanticPolicyRecord>,
    row_records: HashMap<String, SemanticPolicyRecord>,
    account_calls: Mutex<Vec<(Vec<String>, NaiveDate)>>,
    row_calls: Mutex<Vec<(String, Vec<(String, NaiveDate)>)>>,
}

impl RecordingSemanticEligibilityService {
    fn with_records(
        account_records: HashMap<String, SemanticPolicyRecord>,
        row_records: HashMap<String, SemanticPolicyRecord>,
    ) -> Self {
        Self {
            account_records,
            row_records,
            ..Default::default()
        }
    }
}

#[async_trait]
impl AggregateEligibilityServiceTrait for RecordingSemanticEligibilityService {
    async fn eligible_account_ids(
        &self,
        requested_account_ids: &[String],
        as_of: NaiveDate,
    ) -> Result<HashSet<String>> {
        self.account_calls
            .lock()
            .unwrap()
            .push((requested_account_ids.to_vec(), as_of));
        Ok(requested_account_ids
            .iter()
            .filter(|id| {
                self.account_records
                    .get(*id)
                    .is_some_and(|record| record.is_eligible(as_of))
            })
            .cloned()
            .collect())
    }

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>> {
        self.row_calls
            .lock()
            .unwrap()
            .push((entity_type.to_string(), rows.to_vec()));
        Ok(rows
            .iter()
            .filter(|(id, date)| {
                self.row_records
                    .get(id)
                    .is_some_and(|record| record.is_eligible(*date))
            })
            .map(|(id, _)| id.clone())
            .collect())
    }
}

#[derive(Default)]
struct SemanticEligibilityServiceWithErrors {
    account_records: HashMap<String, SemanticPolicyRecord>,
    row_records: HashMap<String, SemanticPolicyRecord>,
    account_calls: Mutex<Vec<(Vec<String>, NaiveDate)>>,
    row_calls: Mutex<Vec<(String, Vec<(String, NaiveDate)>)>>,
    account_error: Option<String>,
    row_error: Option<String>,
}

impl SemanticEligibilityServiceWithErrors {
    fn with_records(
        account_records: HashMap<String, SemanticPolicyRecord>,
        row_records: HashMap<String, SemanticPolicyRecord>,
    ) -> Self {
        Self {
            account_records,
            row_records,
            ..Default::default()
        }
    }
}

#[async_trait]
impl AggregateEligibilityServiceTrait for SemanticEligibilityServiceWithErrors {
    async fn eligible_account_ids(
        &self,
        requested_account_ids: &[String],
        as_of: NaiveDate,
    ) -> Result<HashSet<String>> {
        self.account_calls
            .lock()
            .unwrap()
            .push((requested_account_ids.to_vec(), as_of));
        if let Some(message) = &self.account_error {
            return Err(crate::Error::ConstraintViolation(message.clone()));
        }
        Ok(requested_account_ids
            .iter()
            .filter(|id| {
                self.account_records
                    .get(*id)
                    .is_some_and(|record| record.is_eligible(as_of))
            })
            .cloned()
            .collect())
    }

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>> {
        self.row_calls
            .lock()
            .unwrap()
            .push((entity_type.to_string(), rows.to_vec()));
        if let Some(message) = &self.row_error {
            return Err(crate::Error::ConstraintViolation(message.clone()));
        }
        Ok(rows
            .iter()
            .filter(|(id, date)| {
                self.row_records
                    .get(id)
                    .is_some_and(|record| record.is_eligible(*date))
            })
            .map(|(id, _)| id.clone())
            .collect())
    }
}

fn create_net_worth_service(
    accounts: Vec<Account>,
    assets: Vec<Asset>,
    snapshots: Vec<AccountStateSnapshot>,
    quotes: Vec<Quote>,
) -> NetWorthService {
    create_net_worth_service_with_valuations(accounts, assets, snapshots, quotes, vec![])
}

fn create_net_worth_service_with_valuations(
    accounts: Vec<Account>,
    assets: Vec<Asset>,
    snapshots: Vec<AccountStateSnapshot>,
    quotes: Vec<Quote>,
    valuations: Vec<DailyAccountValuation>,
) -> NetWorthService {
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let effective_accounts = if accounts.is_empty() && !valuations.is_empty() {
        valuations
            .iter()
            .map(|valuation| valuation.account_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|account_id| create_test_account(&account_id, account_types::SECURITIES, "USD"))
            .collect()
    } else {
        accounts
    };

    // Default: all accounts with include_in_net_worth=true are eligible.
    let aggregate_eligibility_service = Arc::new(MockAggregateEligibilityService::allow_all_for(
        &effective_accounts,
        &snapshots,
        &valuations,
    ));

    let account_repo = Arc::new(MockAccountRepository::new(effective_accounts));
    let asset_repo = Arc::new(MockAssetRepository::new(assets));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(snapshots));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(quotes));
    let valuation_repo = Arc::new(MockValuationRepository::new(valuations));
    let fx_service = Arc::new(MockFxService::new("USD"));

    NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        aggregate_eligibility_service,
    )
}

fn create_total_valuation(
    date: NaiveDate,
    total_value: Decimal,
    net_contribution: Decimal,
) -> DailyAccountValuation {
    DailyAccountValuation {
        id: format!("account-1_{}", date),
        account_id: "account-1".to_string(),
        valuation_date: date,
        account_currency: "USD".to_string(),
        base_currency: "USD".to_string(),
        fx_rate_to_base: dec!(1),
        cash_balance: Decimal::ZERO,
        investment_market_value: total_value,
        total_value,
        cost_basis: net_contribution,
        book_basis: net_contribution,
        net_contribution,
        cash_balance_base: Decimal::ZERO,
        investment_market_value_base: total_value,
        total_value_base: total_value,
        cost_basis_base: net_contribution,
        book_basis_base: net_contribution,
        net_contribution_base: net_contribution,
        external_inflow_base: Decimal::ZERO,
        external_outflow_base: Decimal::ZERO,
        external_flow_source: ExternalFlowSource::Unknown,
        performance_eligible_value_base: total_value,
        value_status: ValuationStatus::Complete,
        basis_status: if total_value.is_zero() {
            BasisStatus::NotApplicable
        } else {
            BasisStatus::Complete
        },
        calculated_at: Utc::now(),
    }
}

fn create_account_valuation(
    account_id: &str,
    date: NaiveDate,
    total_value: Decimal,
) -> DailyAccountValuation {
    DailyAccountValuation {
        id: format!("{}_{}", account_id, date),
        account_id: account_id.to_string(),
        valuation_date: date,
        account_currency: "USD".to_string(),
        base_currency: "USD".to_string(),
        fx_rate_to_base: dec!(1),
        cash_balance: total_value,
        investment_market_value: Decimal::ZERO,
        total_value,
        cost_basis: Decimal::ZERO,
        book_basis: total_value,
        net_contribution: Decimal::ZERO,
        cash_balance_base: total_value,
        investment_market_value_base: Decimal::ZERO,
        total_value_base: total_value,
        cost_basis_base: Decimal::ZERO,
        book_basis_base: total_value,
        net_contribution_base: Decimal::ZERO,
        external_inflow_base: Decimal::ZERO,
        external_outflow_base: Decimal::ZERO,
        external_flow_source: ExternalFlowSource::Unknown,
        performance_eligible_value_base: total_value,
        value_status: ValuationStatus::Complete,
        basis_status: BasisStatus::NotApplicable,
        calculated_at: Utc::now(),
    }
}

// Helper to get category value from breakdown
fn get_category_value(response: &NetWorthResponse, category: &str) -> Decimal {
    response
        .assets
        .breakdown
        .iter()
        .find(|b| b.category == category)
        .map(|b| b.value)
        .unwrap_or(Decimal::ZERO)
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_empty_accounts_returns_zero_net_worth() {
    let service = create_net_worth_service(vec![], vec![], vec![], vec![]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, Decimal::ZERO);
    assert_eq!(result.assets.total, Decimal::ZERO);
    assert_eq!(result.liabilities.total, Decimal::ZERO);
    assert_eq!(result.currency, "USD");
}

#[tokio::test]
async fn test_net_worth_filters_snapshot_rows_through_aggregate_policy() {
    let allowed = create_test_account("allowed", "SECURITIES", "USD");
    let denied = create_test_account("denied", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let allowed_snapshot = create_test_snapshot(
        "allowed",
        vec![create_test_position(
            "allowed",
            "AAPL",
            dec!(1),
            dec!(100),
            "USD",
        )],
        HashMap::new(),
    );
    let denied_snapshot = create_test_snapshot(
        "denied",
        vec![create_test_position(
            "denied",
            "AAPL",
            dec!(10),
            dec!(1000),
            "USD",
        )],
        HashMap::new(),
    );
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(100), date, "USD");
    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["allowed", "denied"],
        &[allowed_snapshot.id.as_str()],
        &[],
    ));

    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![allowed, denied])),
        Arc::new(MockAssetRepository::new(vec![asset])),
        Arc::new(MockSnapshotRepository::new(vec![
            allowed_snapshot,
            denied_snapshot,
        ])),
        Arc::new(MockMarketDataRepository::new(vec![quote])),
        Arc::new(MockValuationRepository::new(vec![])),
        Arc::new(MockFxService::new("USD")),
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.assets.total, dec!(100));
    assert_eq!(result.net_worth, dec!(100));
}

#[tokio::test]
async fn test_net_worth_history_filters_valuation_rows_through_aggregate_policy() {
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let allowed_account = create_test_account("allowed", "SECURITIES", "USD");
    let denied_account = create_test_account("denied", "SECURITIES", "USD");
    let allowed_valuation = create_account_valuation("allowed", date, dec!(100));
    let denied_valuation = create_account_valuation("denied", date, dec!(900));
    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["allowed", "denied"],
        &[],
        &[allowed_valuation.id.as_str()],
    ));

    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![
            allowed_account,
            denied_account,
        ])),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(vec![
            allowed_valuation,
            denied_valuation,
        ])),
        Arc::new(MockFxService::new("USD")),
        policy,
    );

    let history = service.get_net_worth_history(date, date).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].total_assets, dec!(100));
    assert_eq!(history[0].net_worth, dec!(100));
}

#[tokio::test]
async fn test_net_worth_excludes_accounts_opted_out_of_net_worth() {
    let included = create_test_account("included", "SECURITIES", "USD");
    let mut excluded = create_test_account("excluded", "SECURITIES", "USD");
    excluded.include_in_net_worth = false;
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let included_snapshot = create_test_snapshot(
        "included",
        vec![create_test_position(
            "included",
            "AAPL",
            dec!(1),
            dec!(10),
            "USD",
        )],
        HashMap::new(),
    );
    let excluded_snapshot = create_test_snapshot(
        "excluded",
        vec![create_test_position(
            "excluded",
            "AAPL",
            dec!(2),
            dec!(10),
            "USD",
        )],
        HashMap::new(),
    );
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(10), date, "USD");

    let service = create_net_worth_service(
        vec![included, excluded],
        vec![asset],
        vec![included_snapshot, excluded_snapshot],
        vec![quote],
    );
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, dec!(10));
    assert_eq!(get_category_value(&result, "investments"), dec!(10));
}

#[tokio::test]
async fn test_single_investment_account() {
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("acc1", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("acc1", vec![position], HashMap::new());
    let quote = create_test_quote(
        "AAPL",
        dec!(185),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // 100 shares * $185 = $18,500
    assert_eq!(result.net_worth, dec!(18500));
    assert_eq!(result.assets.total, dec!(18500));
    assert_eq!(result.liabilities.total, Decimal::ZERO);
    assert_eq!(get_category_value(&result, "investments"), dec!(18500));
}

#[tokio::test]
async fn test_net_worth_uses_stored_investment_valuation_and_keeps_alternatives() {
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let account = create_test_account("account-1", "SECURITIES", "USD");
    let asset = create_test_asset("NFLX", AssetKind::Investment, "USD");
    let property_asset = create_test_asset("PROP-1", AssetKind::Property, "USD");
    let position = create_test_position("account-1", "NFLX", dec!(20), dec!(200), "USD");
    let property_position = create_test_position("account-1", "PROP-1", dec!(1), dec!(5000), "USD");
    let snapshot = create_test_snapshot(
        "account-1",
        vec![position, property_position],
        HashMap::new(),
    );
    let quote = create_test_quote("NFLX", dec!(100), date, "USD");
    let property_quote = create_test_quote("PROP-1", dec!(5000), date, "USD");

    let service = create_net_worth_service_with_valuations(
        vec![account],
        vec![asset, property_asset],
        vec![snapshot],
        vec![quote, property_quote],
        vec![create_total_valuation(date, dec!(20000), Decimal::ZERO)],
    );

    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, dec!(25000));
    assert_eq!(result.assets.total, dec!(25000));
    assert_eq!(get_category_value(&result, "investments"), dec!(20000));
    assert_eq!(get_category_value(&result, "properties"), dec!(5000));
}

#[tokio::test]
async fn test_net_worth_with_liability() {
    // Investment account
    let inv_account = create_test_account("inv1", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("inv1", "AAPL", dec!(100), dec!(15000), "USD");
    let inv_snapshot = create_test_snapshot("inv1", vec![position], HashMap::new());
    let quote = create_test_quote(
        "AAPL",
        dec!(200),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    // Liability account
    let liab_account = create_test_account("liab1", "LIABILITY", "USD");
    let liab_asset = create_test_asset("LIAB-12345", AssetKind::Liability, "USD");
    let liab_position = create_test_position("liab1", "LIAB-12345", dec!(1), dec!(50000), "USD");
    let liab_snapshot = create_test_snapshot("liab1", vec![liab_position], HashMap::new());
    let liab_quote = create_test_quote(
        "LIAB-12345",
        dec!(50000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(
        vec![inv_account, liab_account],
        vec![asset, liab_asset],
        vec![inv_snapshot, liab_snapshot],
        vec![quote, liab_quote],
    );

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Assets: 100 * $200 = $20,000
    // Liabilities: $50,000
    // Net Worth: $20,000 - $50,000 = -$30,000
    assert_eq!(result.assets.total, dec!(20000));
    assert_eq!(result.liabilities.total, dec!(50000));
    assert_eq!(result.net_worth, dec!(-30000));
    assert_eq!(get_category_value(&result, "investments"), dec!(20000));
}

#[tokio::test]
async fn test_property_breakdown() {
    let account = create_test_account("prop1", "PROPERTY", "USD");
    let asset = create_test_asset("PROP-abc123", AssetKind::Property, "USD");
    let position = create_test_position("prop1", "PROP-abc123", dec!(1), dec!(400000), "USD");
    let snapshot = create_test_snapshot("prop1", vec![position], HashMap::new());
    let quote = create_test_quote(
        "PROP-abc123",
        dec!(450000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, dec!(450000));
    assert_eq!(get_category_value(&result, "properties"), dec!(450000));
    assert_eq!(get_category_value(&result, "investments"), Decimal::ZERO);
}

#[tokio::test]
async fn test_cash_included_in_investments() {
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let mut cash = HashMap::new();
    cash.insert("USD".to_string(), dec!(10000));
    let snapshot = create_test_snapshot("acc1", vec![], cash);

    let service = create_net_worth_service(vec![account], vec![], vec![snapshot], vec![]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, dec!(10000));
    // Cash is in the cash category
    assert_eq!(get_category_value(&result, "cash"), dec!(10000));
}

#[tokio::test]
async fn test_credit_card_negative_balance_is_liability() {
    let account = create_test_account("card1", "CREDIT_CARD", "USD");
    let mut cash = HashMap::new();
    cash.insert("USD".to_string(), dec!(-1250));
    let snapshot = create_test_snapshot("card1", vec![], cash);

    let service = create_net_worth_service(vec![account], vec![], vec![snapshot], vec![]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.assets.total, Decimal::ZERO);
    assert_eq!(result.liabilities.total, dec!(1250));
    assert_eq!(result.net_worth, dec!(-1250));
    assert_eq!(result.stale_assets.len(), 0);
}

#[tokio::test]
async fn test_credit_card_positive_balance_is_cash() {
    let account = create_test_account("card1", "CREDIT_CARD", "USD");
    let mut cash = HashMap::new();
    cash.insert("USD".to_string(), dec!(25));
    let snapshot = create_test_snapshot("card1", vec![], cash);

    let service = create_net_worth_service(vec![account], vec![], vec![snapshot], vec![]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.assets.total, dec!(25));
    assert_eq!(result.liabilities.total, Decimal::ZERO);
    assert_eq!(result.net_worth, dec!(25));
    assert_eq!(get_category_value(&result, "cash"), dec!(25));
}

#[tokio::test]
async fn test_credit_card_multi_currency_nets_before_liability_split() {
    let account = create_test_account("card1", "CREDIT_CARD", "USD");
    let mut cash = HashMap::new();
    cash.insert("USD".to_string(), dec!(-500));
    cash.insert("EUR".to_string(), dec!(200));
    let snapshot = create_test_snapshot("card1", vec![], cash);
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    let service = create_net_worth_service_with_valuations(
        vec![account],
        vec![],
        vec![snapshot],
        vec![],
        vec![create_account_valuation("card1", date, dec!(-300))],
    );

    let result = service.get_net_worth(date).await.unwrap();
    assert_eq!(result.assets.total, Decimal::ZERO);
    assert_eq!(result.liabilities.total, dec!(300));
    assert_eq!(result.net_worth, dec!(-300));
    assert_eq!(get_category_value(&result, "cash"), Decimal::ZERO);

    let history = service.get_net_worth_history(date, date).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].total_assets, Decimal::ZERO);
    assert_eq!(history[0].total_liabilities, dec!(300));
    assert_eq!(history[0].net_worth, dec!(-300));
}

#[tokio::test]
async fn test_staleness_detection() {
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("acc1", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("acc1", vec![position], HashMap::new());

    // Quote from 100 days ago
    let old_date = NaiveDate::from_ymd_opt(2023, 10, 7).unwrap();
    let quote = create_test_quote("AAPL", dec!(150), old_date, "USD");

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Should detect stale asset (>90 days)
    assert_eq!(result.stale_assets.len(), 1);
    assert_eq!(result.stale_assets[0].asset_id, "AAPL");
    assert!(result.stale_assets[0].days_stale > 90);
    assert_eq!(result.oldest_valuation_date, Some(old_date));
}

#[tokio::test]
async fn test_expired_option_excluded_from_stale_valuations() {
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let stock = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let mut option = create_test_asset("TSLA200117C00397500", AssetKind::Investment, "USD");
    option.instrument_type = Some(InstrumentType::Option);
    option.instrument_symbol = Some("TSLA200117C00397500".to_string());
    option.metadata = Some(json!({
        "option": {
            "expiration": "2020-01-17"
        }
    }));

    let stock_position = create_test_position("acc1", "AAPL", dec!(100), dec!(15000), "USD");
    let option_position =
        create_test_position("acc1", "TSLA200117C00397500", dec!(1), dec!(500), "USD");
    let snapshot = create_test_snapshot(
        "acc1",
        vec![stock_position, option_position],
        HashMap::new(),
    );

    let old_date = NaiveDate::from_ymd_opt(2023, 10, 7).unwrap();
    let stock_quote = create_test_quote("AAPL", dec!(150), old_date, "USD");
    let option_quote = create_test_quote("TSLA200117C00397500", dec!(25), old_date, "USD");

    let service = create_net_worth_service(
        vec![account],
        vec![stock, option],
        vec![snapshot],
        vec![stock_quote, option_quote],
    );

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.assets.total, dec!(15000));
    assert_eq!(result.stale_assets.len(), 1);
    assert_eq!(result.stale_assets[0].asset_id, "AAPL");
    assert_eq!(result.oldest_valuation_date, Some(old_date));
}

#[tokio::test]
async fn test_zero_value_liability_not_stale() {
    // A paid-off liability sitting at $0 has nothing to update, so an old
    // zero-value valuation must not be flagged as stale.
    let liab_asset = create_test_asset("LIAB-mortgage", AssetKind::Liability, "USD");

    // Old quote (100 days ago) priced at zero → market value of zero.
    let old_date = NaiveDate::from_ymd_opt(2023, 10, 7).unwrap();
    let quote = create_test_quote("LIAB-mortgage", dec!(0), old_date, "USD");

    let service = create_net_worth_service(vec![], vec![liab_asset], vec![], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Zero-value liability is excluded from staleness even though it is >90 days old.
    assert_eq!(result.stale_assets.len(), 0);
    // It is also excluded from the oldest-valuation-date signal the widget uses,
    // so the "stale valuations" banner stays hidden.
    assert_eq!(result.oldest_valuation_date, None);
}

#[tokio::test]
async fn test_zero_balance_credit_card_remains_visible_in_liabilities() {
    let card = create_test_account("card-zero", "CREDIT_CARD", "USD");
    let snapshot = create_test_snapshot("card-zero", vec![], HashMap::new());
    let service = create_net_worth_service(vec![card], vec![], vec![snapshot], vec![]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();
    let row = result
        .liabilities
        .breakdown
        .iter()
        .find(|item| item.asset_id.as_deref() == Some("CREDIT_CARD:card-zero"))
        .expect("verified credit-card account should remain visible");
    assert_eq!(row.name, "Test Account card-zero");
    assert_eq!(row.value, dec!(0));
}

#[tokio::test]
async fn test_zero_value_investment_still_stale() {
    // A zero value on a non-liability asset is more likely a data issue (e.g. a
    // bad quote), so it must still be flagged as stale to prompt a refresh.
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("acc1", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("acc1", vec![position], HashMap::new());

    // Old quote (100 days ago) priced at zero → market value of zero.
    let old_date = NaiveDate::from_ymd_opt(2023, 10, 7).unwrap();
    let quote = create_test_quote("AAPL", dec!(0), old_date, "USD");

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.stale_assets.len(), 1);
    assert_eq!(result.stale_assets[0].asset_id, "AAPL");
}

#[tokio::test]
async fn test_multiple_asset_categories() {
    // Securities account
    let sec_account = create_test_account("sec1", "SECURITIES", "USD");
    let sec_asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let sec_position = create_test_position("sec1", "AAPL", dec!(50), dec!(7500), "USD");
    let sec_snapshot = create_test_snapshot("sec1", vec![sec_position], HashMap::new());
    let sec_quote = create_test_quote(
        "AAPL",
        dec!(200),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    // Property account
    let prop_account = create_test_account("prop1", "PROPERTY", "USD");
    let prop_asset = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let prop_position = create_test_position("prop1", "PROP-house", dec!(1), dec!(300000), "USD");
    let prop_snapshot = create_test_snapshot("prop1", vec![prop_position], HashMap::new());
    let prop_quote = create_test_quote(
        "PROP-house",
        dec!(350000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    // Vehicle account
    let veh_account = create_test_account("veh1", "VEHICLE", "USD");
    let veh_asset = create_test_asset("VEH-car", AssetKind::Vehicle, "USD");
    let veh_position = create_test_position("veh1", "VEH-car", dec!(1), dec!(35000), "USD");
    let veh_snapshot = create_test_snapshot("veh1", vec![veh_position], HashMap::new());
    let veh_quote = create_test_quote(
        "VEH-car",
        dec!(30000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    // Liability account
    let liab_account = create_test_account("liab1", "LIABILITY", "USD");
    let liab_asset = create_test_asset("LIAB-mortgage", AssetKind::Liability, "USD");
    let liab_position =
        create_test_position("liab1", "LIAB-mortgage", dec!(1), dec!(200000), "USD");
    let liab_snapshot = create_test_snapshot("liab1", vec![liab_position], HashMap::new());
    let liab_quote = create_test_quote(
        "LIAB-mortgage",
        dec!(200000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(
        vec![sec_account, prop_account, veh_account, liab_account],
        vec![sec_asset, prop_asset, veh_asset, liab_asset],
        vec![sec_snapshot, prop_snapshot, veh_snapshot, liab_snapshot],
        vec![sec_quote, prop_quote, veh_quote, liab_quote],
    );

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Investments: 50 * $200 = $10,000
    // Properties: $350,000
    // Vehicles: $30,000
    // Liabilities: $200,000
    // Total Assets: $10,000 + $350,000 + $30,000 = $390,000
    // Net Worth: $390,000 - $200,000 = $190,000

    assert_eq!(get_category_value(&result, "investments"), dec!(10000));
    assert_eq!(get_category_value(&result, "properties"), dec!(350000));
    assert_eq!(get_category_value(&result, "vehicles"), dec!(30000));
    assert_eq!(result.assets.total, dec!(390000));
    assert_eq!(result.liabilities.total, dec!(200000));
    assert_eq!(result.net_worth, dec!(190000));
}

#[tokio::test]
async fn test_precious_metals_category() {
    let account = create_test_account("prec1", "PRECIOUS", "USD");
    let asset = create_test_asset("PREC-gold", AssetKind::PreciousMetal, "USD");
    // 10 oz of gold
    let position = create_test_position("prec1", "PREC-gold", dec!(10), dec!(18000), "USD");
    let snapshot = create_test_snapshot("prec1", vec![position], HashMap::new());
    // Gold at $2000/oz
    let quote = create_test_quote(
        "PREC-gold",
        dec!(2000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // 10 oz * $2000 = $20,000
    assert_eq!(result.net_worth, dec!(20000));
    assert_eq!(get_category_value(&result, "preciousMetals"), dec!(20000));
}

#[tokio::test]
async fn test_collectibles_category() {
    let account = create_test_account("coll1", "COLLECTIBLE", "USD");
    let asset = create_test_asset("COLL-art", AssetKind::Collectible, "USD");
    let position = create_test_position("coll1", "COLL-art", dec!(1), dec!(50000), "USD");
    let snapshot = create_test_snapshot("coll1", vec![position], HashMap::new());
    let quote = create_test_quote(
        "COLL-art",
        dec!(75000),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    assert_eq!(result.net_worth, dec!(75000));
    assert_eq!(get_category_value(&result, "collectibles"), dec!(75000));
}

#[tokio::test]
async fn test_no_quote_falls_back_to_cost_basis() {
    let account = create_test_account("acc1", "SECURITIES", "USD");
    let asset = create_test_asset("NEWSTOCK", AssetKind::Investment, "USD");
    // Position with $10,000 cost basis, 100 shares -> implied price $100/share
    let position = create_test_position("acc1", "NEWSTOCK", dec!(100), dec!(10000), "USD");
    let snapshot = create_test_snapshot("acc1", vec![position], HashMap::new());
    // No quotes available

    let service = create_net_worth_service(
        vec![account],
        vec![asset],
        vec![snapshot],
        vec![], // Empty quotes
    );

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Should use cost basis: 100 * ($10,000/100) = $10,000
    assert_eq!(result.net_worth, dec!(10000));
}

// ============================================================================
// Net Worth History Tests
// ============================================================================

#[tokio::test]
async fn test_history_basic_portfolio_with_alt_assets() {
    // Setup: Portfolio + Property + Liability over 5 days
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
    let d4 = NaiveDate::from_ymd_opt(2024, 1, 4).unwrap();
    let d5 = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();

    // Portfolio valuations
    let valuations = vec![
        create_total_valuation(d1, dec!(100000), dec!(95000)), // $5K gain
        create_total_valuation(d2, dec!(101000), dec!(95000)), // $6K gain
        create_total_valuation(d3, dec!(102000), dec!(96000)), // $6K gain (deposited $1K)
        create_total_valuation(d4, dec!(103000), dec!(96000)), // $7K gain
        create_total_valuation(d5, dec!(104000), dec!(96000)), // $8K gain
    ];

    // Property asset
    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let property_quotes = vec![
        create_test_quote("PROP-house", dec!(500000), d1, "USD"),
        create_test_quote("PROP-house", dec!(505000), d5, "USD"), // Appreciated
    ];

    // Liability asset
    let liability = create_test_asset("LIAB-mortgage", AssetKind::Liability, "USD");
    let liability_quotes = vec![
        create_test_quote("LIAB-mortgage", dec!(300000), d1, "USD"),
        create_test_quote("LIAB-mortgage", dec!(298000), d5, "USD"), // Paid down
    ];

    let all_quotes = [property_quotes, liability_quotes].concat();

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property, liability],
        vec![],
        all_quotes,
        valuations,
    );

    let history = service.get_net_worth_history(d1, d5).await.unwrap();

    assert_eq!(history.len(), 5, "Should have 5 data points");

    // Day 1: Portfolio 100K + Property 500K - Liability 300K = 300K
    assert_eq!(history[0].date, d1);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].alternative_assets_value, dec!(500000));
    assert_eq!(history[0].total_liabilities, dec!(300000));
    assert_eq!(history[0].net_worth, dec!(300000));
    assert_eq!(history[0].net_contribution, dec!(95000));
    assert_eq!(history[0].breakdown.get("investments"), Some(&dec!(100000)));
    assert_eq!(history[0].breakdown.get("properties"), Some(&dec!(500000)));
    assert_eq!(
        history[0].breakdown.get("LIAB-mortgage"),
        Some(&dec!(300000))
    );

    // Day 5: Portfolio 104K + Property 505K - Liability 298K = 311K
    let last = &history[4];
    assert_eq!(last.date, d5);
    assert_eq!(last.portfolio_value, dec!(104000));
    assert_eq!(last.alternative_assets_value, dec!(505000));
    assert_eq!(last.total_liabilities, dec!(298000));
    assert_eq!(last.net_worth, dec!(311000));
    assert_eq!(last.net_contribution, dec!(96000));
    assert_eq!(last.breakdown.get("investments"), Some(&dec!(104000)));
    assert_eq!(last.breakdown.get("properties"), Some(&dec!(505000)));
    assert_eq!(last.breakdown.get("LIAB-mortgage"), Some(&dec!(298000)));

    // Verify gain calculation would work correctly:
    // Portfolio gain = (104000 - 96000) - (100000 - 95000) = 8000 - 5000 = 3000
    // Alt asset gain = 505000 - 500000 = 5000
    // Liability reduction = 300000 - 298000 = 2000
    // Total gain = 3000 + 5000 + 2000 = 10000
    let first = &history[0];
    let portfolio_gain = (last.portfolio_value - last.net_contribution)
        - (first.portfolio_value - first.net_contribution);
    let alt_asset_gain = last.alternative_assets_value - first.alternative_assets_value;
    let liability_reduction = first.total_liabilities - last.total_liabilities;
    let total_gain = portfolio_gain + alt_asset_gain + liability_reduction;
    assert_eq!(total_gain, dec!(10000));
}

#[tokio::test]
async fn test_history_portfolio_only_no_alt_assets() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();

    let valuations = vec![
        create_total_valuation(d1, dec!(50000), dec!(48000)),
        create_total_valuation(d2, dec!(51000), dec!(48000)),
        create_total_valuation(d3, dec!(52000), dec!(48000)),
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![], // No alternative assets
        vec![],
        vec![], // No quotes
        valuations,
    );

    let history = service.get_net_worth_history(d1, d3).await.unwrap();

    assert_eq!(history.len(), 3);
    assert_eq!(history[0].portfolio_value, dec!(50000));
    assert_eq!(history[0].alternative_assets_value, Decimal::ZERO);
    assert_eq!(history[0].total_liabilities, Decimal::ZERO);
    assert_eq!(history[0].net_worth, dec!(50000));

    assert_eq!(history[2].net_worth, dec!(52000));
}

#[tokio::test]
async fn test_history_carries_forward_missing_account_valuation() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();

    let valuations = vec![
        create_total_valuation(d1, dec!(100), dec!(100)),
        create_total_valuation(d2, dec!(110), dec!(100)),
        create_total_valuation(d3, dec!(120), dec!(100)),
        create_account_valuation("account-2", d1, dec!(50)),
        create_account_valuation("account-2", d2, dec!(55)),
    ];
    let accounts = vec![
        create_test_account("account-1", "SECURITIES", "USD"),
        create_test_account("account-2", "CASH", "USD"),
    ];

    let service =
        create_net_worth_service_with_valuations(accounts, vec![], vec![], vec![], valuations);
    let history = service.get_net_worth_history(d1, d3).await.unwrap();

    assert_eq!(history.len(), 3);
    assert_eq!(history[2].portfolio_value, dec!(175));
    assert_eq!(history[2].net_worth, dec!(175));
}

#[tokio::test]
async fn test_history_credit_card_balance_is_split_to_liabilities() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let valuations = vec![
        create_total_valuation(d1, dec!(100000), dec!(100000)),
        create_account_valuation("card1", d1, dec!(-1250)),
    ];
    let portfolio_account = create_test_account("account-1", "SECURITIES", "USD");
    let card = create_test_account("card1", "CREDIT_CARD", "USD");

    let service = create_net_worth_service_with_valuations(
        vec![portfolio_account, card],
        vec![],
        vec![],
        vec![],
        valuations,
    );

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].total_assets, dec!(100000));
    assert_eq!(history[0].total_liabilities, dec!(1250));
    assert_eq!(history[0].net_worth, dec!(98750));
    assert_eq!(history[0].breakdown.get("investments"), Some(&dec!(100000)));
    assert_eq!(
        history[0].breakdown.get("CREDIT_CARD:card1"),
        Some(&dec!(1250))
    );
}

#[tokio::test]
async fn test_history_credit_card_balance_before_first_portfolio_date_is_seeded() {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let card_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let first_portfolio_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let valuations = vec![
        create_total_valuation(first_portfolio_date, dec!(100000), dec!(100000)),
        create_account_valuation("card1", card_date, dec!(-500)),
    ];
    let portfolio_account = create_test_account("account-1", "SECURITIES", "USD");
    let card = create_test_account("card1", "CREDIT_CARD", "USD");

    let service = create_net_worth_service_with_valuations(
        vec![portfolio_account, card],
        vec![],
        vec![],
        vec![],
        valuations,
    );

    let history = service
        .get_net_worth_history(start, first_portfolio_date)
        .await
        .unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, first_portfolio_date);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].total_liabilities, dec!(500));
    assert_eq!(history[0].net_worth, dec!(99500));
}

#[tokio::test]
async fn test_history_credit_card_without_total_stays_liability_only() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let account = create_test_account("card1", "CREDIT_CARD", "USD");
    let valuations = vec![create_account_valuation("card1", d1, dec!(-500))];

    let service =
        create_net_worth_service_with_valuations(vec![account], vec![], vec![], vec![], valuations);

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].total_assets, Decimal::ZERO);
    assert_eq!(history[0].total_liabilities, dec!(500));
    assert_eq!(history[0].net_worth, dec!(-500));
}

#[tokio::test]
async fn test_history_credit_card_positive_balance_without_total_is_cash_asset() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let account = create_test_account("card1", "CREDIT_CARD", "USD");
    let valuations = vec![create_account_valuation("card1", d1, dec!(25))];

    let service =
        create_net_worth_service_with_valuations(vec![account], vec![], vec![], vec![], valuations);

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].total_assets, dec!(25));
    assert_eq!(history[0].total_liabilities, Decimal::ZERO);
    assert_eq!(history[0].net_worth, dec!(25));
}

#[tokio::test]
async fn test_history_credit_card_liability_carries_forward_between_dates() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();

    // Portfolio holds steady at $100K. Card balance changes on d1 and d2, then
    // has no valuation on d3 (the balance should carry forward from d2).
    let valuations = vec![
        create_total_valuation(d1, dec!(100000), dec!(100000)),
        create_total_valuation(d2, dec!(100000), dec!(100000)),
        create_total_valuation(d3, dec!(100000), dec!(100000)),
        create_account_valuation("card1", d1, dec!(-500)),
        create_account_valuation("card1", d2, dec!(-1200)),
    ];
    let portfolio_account = create_test_account("account-1", "SECURITIES", "USD");
    let card = create_test_account("card1", "CREDIT_CARD", "USD");

    let service = create_net_worth_service_with_valuations(
        vec![portfolio_account, card],
        vec![],
        vec![],
        vec![],
        valuations,
    );

    let history = service.get_net_worth_history(d1, d3).await.unwrap();

    assert_eq!(history.len(), 3);

    assert_eq!(history[0].date, d1);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].total_liabilities, dec!(500));
    assert_eq!(history[0].net_worth, dec!(99500));

    assert_eq!(history[1].date, d2);
    assert_eq!(history[1].total_liabilities, dec!(1200));
    assert_eq!(history[1].net_worth, dec!(98800));

    // d3 has no new card valuation: liability carries forward from d2.
    assert_eq!(history[2].date, d3);
    assert_eq!(history[2].portfolio_value, dec!(100000));
    assert_eq!(history[2].total_liabilities, dec!(1200));
    assert_eq!(history[2].net_worth, dec!(98800));
}

#[tokio::test]
async fn test_history_mixed_securities_cash_and_credit_card() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    // Securities ($100K) + cash ($5K) both count toward portfolio_value; the
    // credit card ($1,250 owed) is the only account treated as a liability.
    let valuations = vec![
        create_total_valuation(d1, dec!(100000), dec!(100000)),
        create_account_valuation("cash-1", d1, dec!(5000)),
        create_account_valuation("card1", d1, dec!(-1250)),
    ];
    let securities = create_test_account("account-1", "SECURITIES", "USD");
    let cash = create_test_account("cash-1", "CASH", "USD");
    let card = create_test_account("card1", "CREDIT_CARD", "USD");

    let service = create_net_worth_service_with_valuations(
        vec![securities, cash, card],
        vec![],
        vec![],
        vec![],
        valuations,
    );

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].portfolio_value, dec!(105000));
    assert_eq!(history[0].total_assets, dec!(105000));
    assert_eq!(history[0].total_liabilities, dec!(1250));
    assert_eq!(history[0].net_worth, dec!(103750));
    assert_eq!(history[0].breakdown.get("investments"), Some(&dec!(100000)));
    assert_eq!(history[0].breakdown.get("cash"), Some(&dec!(5000)));
    assert_eq!(
        history[0].breakdown.get("CREDIT_CARD:card1"),
        Some(&dec!(1250))
    );
}

#[tokio::test]
async fn test_history_alt_asset_quote_after_range_start_before_first_portfolio_date_seeds_first_point(
) {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let quote_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let first_portfolio_date = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let valuations = vec![create_total_valuation(
        first_portfolio_date,
        dec!(100000),
        dec!(100000),
    )];
    let quotes = vec![create_test_quote(
        "PROP-house",
        dec!(450000),
        quote_date,
        "USD",
    )];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property],
        vec![],
        quotes,
        valuations,
    );

    let history = service
        .get_net_worth_history(start, first_portfolio_date)
        .await
        .unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, first_portfolio_date);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].alternative_assets_value, dec!(450000));
    assert_eq!(history[0].net_worth, dec!(550000));
}

#[tokio::test]
async fn test_history_includes_closed_non_archived_accounts() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let closed_account =
        create_test_account_with_archive_state("closed-1", "SECURITIES", "USD", false, false);
    let mut valuation = create_total_valuation(d1, dec!(50000), dec!(48000));
    valuation.id = "closed-1_2024-01-01".to_string();
    valuation.account_id = "closed-1".to_string();

    let service = create_net_worth_service_with_valuations(
        vec![closed_account],
        vec![],
        vec![],
        vec![],
        vec![valuation],
    );

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].portfolio_value, dec!(50000));
    assert_eq!(history[0].net_worth, dec!(50000));
}

#[tokio::test]
async fn test_history_alt_assets_only_no_portfolio() {
    // Edge case: user only has alternative assets, no portfolio
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let quotes = vec![
        create_test_quote("PROP-house", dec!(400000), d1, "USD"),
        create_test_quote("PROP-house", dec!(410000), d2, "USD"),
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property],
        vec![],
        quotes,
        vec![], // No portfolio valuations
    );

    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].portfolio_value, Decimal::ZERO);
    assert_eq!(history[0].alternative_assets_value, dec!(400000));
    assert_eq!(history[0].net_worth, dec!(400000));
    assert_eq!(history[0].net_contribution, Decimal::ZERO);

    assert_eq!(history[1].alternative_assets_value, dec!(410000));
    assert_eq!(history[1].net_worth, dec!(410000));
}

#[tokio::test]
async fn test_history_forward_fill_alt_assets() {
    // Alt asset has quote on day 1 and day 5 only
    // Days 2-4 should forward-fill from day 1
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
    let d4 = NaiveDate::from_ymd_opt(2024, 1, 4).unwrap();
    let d5 = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();

    // Portfolio has data every day
    let valuations = vec![
        create_total_valuation(d1, dec!(10000), dec!(10000)),
        create_total_valuation(d2, dec!(10100), dec!(10000)),
        create_total_valuation(d3, dec!(10200), dec!(10000)),
        create_total_valuation(d4, dec!(10300), dec!(10000)),
        create_total_valuation(d5, dec!(10400), dec!(10000)),
    ];

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    // Only quotes on d1 and d5
    let quotes = vec![
        create_test_quote("PROP-house", dec!(200000), d1, "USD"),
        create_test_quote("PROP-house", dec!(210000), d5, "USD"),
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property],
        vec![],
        quotes,
        valuations,
    );

    let history = service.get_net_worth_history(d1, d5).await.unwrap();

    assert_eq!(history.len(), 5);

    // Days 2-4 should have forward-filled property value of 200000
    assert_eq!(history[1].alternative_assets_value, dec!(200000));
    assert_eq!(history[2].alternative_assets_value, dec!(200000));
    assert_eq!(history[3].alternative_assets_value, dec!(200000));

    // Day 5 should have updated property value
    assert_eq!(history[4].alternative_assets_value, dec!(210000));
}

#[tokio::test]
async fn test_history_starts_from_first_portfolio_date() {
    // Alt assets have data before portfolio starts
    // History should only start from first portfolio date
    let d_before = NaiveDate::from_ymd_opt(2023, 12, 1).unwrap();
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();

    // Portfolio only starts on d1
    let valuations = vec![
        create_total_valuation(d1, dec!(50000), dec!(50000)),
        create_total_valuation(d2, dec!(51000), dec!(50000)),
    ];

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    // Property has quote before portfolio started
    let quotes = vec![
        create_test_quote("PROP-house", dec!(300000), d_before, "USD"),
        create_test_quote("PROP-house", dec!(310000), d2, "USD"),
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property],
        vec![],
        quotes,
        valuations,
    );

    // Query includes d_before, but history should start from d1
    let history = service.get_net_worth_history(d_before, d2).await.unwrap();

    // Should only have 2 points (d1 and d2), not 3
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].date, d1);

    // d1 should use forward-filled property value from d_before
    assert_eq!(history[0].portfolio_value, dec!(50000));
    assert_eq!(history[0].alternative_assets_value, dec!(300000));
    assert_eq!(history[0].net_worth, dec!(350000));
}

#[tokio::test]
async fn test_history_empty_range() {
    // Query range before any data exists
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 5).unwrap();
    let d_after = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();

    let valuations = vec![create_total_valuation(d_after, dec!(50000), dec!(50000))];

    let service =
        create_net_worth_service_with_valuations(vec![], vec![], vec![], vec![], valuations);

    // Query before any data
    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert!(history.is_empty(), "Should return empty history");
}

#[tokio::test]
async fn test_history_single_day() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    let valuations = vec![create_total_valuation(d1, dec!(100000), dec!(90000))];

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let quotes = vec![create_test_quote("PROP-house", dec!(500000), d1, "USD")];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property],
        vec![],
        quotes,
        valuations,
    );

    let history = service.get_net_worth_history(d1, d1).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, d1);
    assert_eq!(history[0].portfolio_value, dec!(100000));
    assert_eq!(history[0].alternative_assets_value, dec!(500000));
    assert_eq!(history[0].net_worth, dec!(600000));
    assert_eq!(history[0].net_contribution, dec!(90000));
}

#[tokio::test]
async fn test_history_liability_reduction_is_positive_gain() {
    // Test that paying down debt shows as positive contribution to net worth
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();

    let valuations = vec![
        create_total_valuation(d1, dec!(50000), dec!(50000)), // No portfolio gain
        create_total_valuation(d2, dec!(50000), dec!(50000)),
    ];

    let liability = create_test_asset("LIAB-mortgage", AssetKind::Liability, "USD");
    let quotes = vec![
        create_test_quote("LIAB-mortgage", dec!(200000), d1, "USD"),
        create_test_quote("LIAB-mortgage", dec!(195000), d2, "USD"), // Paid $5K
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![liability],
        vec![],
        quotes,
        valuations,
    );

    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert_eq!(history.len(), 2);

    // Day 1: Net worth = 50K - 200K = -150K
    assert_eq!(history[0].net_worth, dec!(-150000));

    // Day 31: Net worth = 50K - 195K = -145K
    assert_eq!(history[1].net_worth, dec!(-145000));

    // Gain from liability reduction
    let liability_reduction = history[0].total_liabilities - history[1].total_liabilities;
    assert_eq!(liability_reduction, dec!(5000));
}

#[tokio::test]
async fn test_history_contribution_adjusted_gain() {
    // Verify that deposits don't count as gain
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    // Day 1: 100K portfolio, 95K contributed (5K gain)
    // Day 15: 150K portfolio, 140K contributed (10K gain)
    // User deposited 45K, market gained 5K
    let valuations = vec![
        create_total_valuation(d1, dec!(100000), dec!(95000)),
        create_total_valuation(d2, dec!(150000), dec!(140000)),
    ];

    let service =
        create_net_worth_service_with_valuations(vec![], vec![], vec![], vec![], valuations);

    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert_eq!(history.len(), 2);

    let first = &history[0];
    let last = &history[1];

    // Net worth increased by 50K
    let net_worth_change = last.net_worth - first.net_worth;
    assert_eq!(net_worth_change, dec!(50000));

    // But contribution-adjusted gain is only 5K
    let portfolio_gain = (last.portfolio_value - last.net_contribution)
        - (first.portfolio_value - first.net_contribution);
    assert_eq!(portfolio_gain, dec!(5000));
}

#[tokio::test]
async fn test_history_multiple_alt_assets() {
    // Multiple alternative assets, each with different update schedules
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();

    let valuations = vec![
        create_total_valuation(d1, dec!(10000), dec!(10000)),
        create_total_valuation(d2, dec!(10000), dec!(10000)),
        create_total_valuation(d3, dec!(10000), dec!(10000)),
    ];

    let property = create_test_asset("PROP-house", AssetKind::Property, "USD");
    let gold = create_test_asset("PREC-gold", AssetKind::PreciousMetal, "USD");
    let mortgage = create_test_asset("LIAB-mortgage", AssetKind::Liability, "USD");

    let quotes = vec![
        // Property: updates d1 and d3
        create_test_quote("PROP-house", dec!(500000), d1, "USD"),
        create_test_quote("PROP-house", dec!(510000), d3, "USD"),
        // Gold: updates d1 and d2
        create_test_quote("PREC-gold", dec!(5000), d1, "USD"),
        create_test_quote("PREC-gold", dec!(5500), d2, "USD"),
        // Mortgage: only d1
        create_test_quote("LIAB-mortgage", dec!(200000), d1, "USD"),
    ];

    let service = create_net_worth_service_with_valuations(
        vec![],
        vec![property, gold, mortgage],
        vec![],
        quotes,
        valuations,
    );

    let history = service.get_net_worth_history(d1, d3).await.unwrap();

    assert_eq!(history.len(), 3);

    // Day 1: 10K + 500K + 5K - 200K = 315K
    assert_eq!(history[0].portfolio_value, dec!(10000));
    assert_eq!(history[0].alternative_assets_value, dec!(505000)); // 500K + 5K
    assert_eq!(history[0].total_liabilities, dec!(200000));
    assert_eq!(history[0].net_worth, dec!(315000));
    assert_eq!(history[0].breakdown.get("investments"), Some(&dec!(10000)));
    assert_eq!(history[0].breakdown.get("properties"), Some(&dec!(500000)));
    assert_eq!(
        history[0].breakdown.get("preciousMetals"),
        Some(&dec!(5000))
    );
    assert_eq!(
        history[0].breakdown.get("LIAB-mortgage"),
        Some(&dec!(200000))
    );

    // Day 2: 10K + 500K + 5.5K - 200K = 315.5K (gold updated, property forward-filled)
    assert_eq!(history[1].alternative_assets_value, dec!(505500)); // 500K + 5.5K
    assert_eq!(history[1].net_worth, dec!(315500));

    // Day 3: 10K + 510K + 5.5K - 200K = 325.5K (property updated, gold forward-filled)
    assert_eq!(history[2].alternative_assets_value, dec!(515500)); // 510K + 5.5K
    assert_eq!(history[2].net_worth, dec!(325500));
    assert_eq!(history[2].breakdown.get("properties"), Some(&dec!(510000)));
    assert_eq!(
        history[2].breakdown.get("preciousMetals"),
        Some(&dec!(5500))
    );
}

// ============================================================================
// Archive Behavior Tests
// ============================================================================

fn create_test_account_with_archive_state(
    id: &str,
    account_type: &str,
    currency: &str,
    is_active: bool,
    is_archived: bool,
) -> Account {
    let now = Utc::now().naive_utc();
    Account {
        include_in_net_worth: true,
        name_source: "provider".to_string(),
        id: id.to_string(),
        name: format!("Test Account {}", id),
        account_type: account_type.to_string(),
        group: None,
        currency: currency.to_string(),
        is_default: false,
        is_active,
        created_at: now,
        updated_at: now,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        is_archived,
        tracking_mode: crate::accounts::TrackingMode::NotSet,
    }
}

#[tokio::test]
async fn test_closed_account_in_net_worth_when_not_archived() {
    // Account is closed (is_active=false) but NOT archived (is_archived=false)
    // Should be included in net worth calculations
    let account =
        create_test_account_with_archive_state("closed_acc", "SECURITIES", "USD", false, false);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("closed_acc", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("closed_acc", vec![position], HashMap::new());
    let quote = create_test_quote(
        "AAPL",
        dec!(200),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Closed but non-archived account should be included
    // 100 shares * $200 = $20,000
    assert_eq!(result.net_worth, dec!(20000));
    assert_eq!(result.assets.total, dec!(20000));
}

#[tokio::test]
async fn test_archived_account_excluded_from_net_worth() {
    // Account is archived (is_archived=true)
    // Should be excluded from net worth calculations
    let account =
        create_test_account_with_archive_state("archived_acc", "SECURITIES", "USD", true, true);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("archived_acc", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("archived_acc", vec![position], HashMap::new());
    let quote = create_test_quote(
        "AAPL",
        dec!(200),
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(vec![account], vec![asset], vec![snapshot], vec![quote]);

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let result = service.get_net_worth(date).await.unwrap();

    // Archived account should be excluded - net worth should be zero
    assert_eq!(result.net_worth, Decimal::ZERO);
    assert_eq!(result.assets.total, Decimal::ZERO);
}

#[tokio::test]
async fn test_net_worth_with_mixed_account_states() {
    // Setup:
    //   - Account A: is_active=true, is_archived=false (active) - $10,000
    //   - Account B: is_active=false, is_archived=false (closed but not archived) - $5,000
    //   - Account C: is_active=true, is_archived=true (active but archived) - $15,000
    //   - Account D: is_active=false, is_archived=true (closed and archived) - $20,000
    // Expected: Only A ($10K) and B ($5K) contribute = $15K total

    let acc_a = create_test_account_with_archive_state("acc_a", "SECURITIES", "USD", true, false);
    let acc_b = create_test_account_with_archive_state("acc_b", "SECURITIES", "USD", false, false);
    let acc_c = create_test_account_with_archive_state("acc_c", "SECURITIES", "USD", true, true);
    let acc_d = create_test_account_with_archive_state("acc_d", "SECURITIES", "USD", false, true);

    let asset_a = create_test_asset("ASSET_A", AssetKind::Investment, "USD");
    let asset_b = create_test_asset("ASSET_B", AssetKind::Investment, "USD");
    let asset_c = create_test_asset("ASSET_C", AssetKind::Investment, "USD");
    let asset_d = create_test_asset("ASSET_D", AssetKind::Investment, "USD");

    let pos_a = create_test_position("acc_a", "ASSET_A", dec!(100), dec!(10000), "USD");
    let pos_b = create_test_position("acc_b", "ASSET_B", dec!(50), dec!(5000), "USD");
    let pos_c = create_test_position("acc_c", "ASSET_C", dec!(150), dec!(15000), "USD");
    let pos_d = create_test_position("acc_d", "ASSET_D", dec!(200), dec!(20000), "USD");

    let snap_a = create_test_snapshot("acc_a", vec![pos_a], HashMap::new());
    let snap_b = create_test_snapshot("acc_b", vec![pos_b], HashMap::new());
    let snap_c = create_test_snapshot("acc_c", vec![pos_c], HashMap::new());
    let snap_d = create_test_snapshot("acc_d", vec![pos_d], HashMap::new());

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote_a = create_test_quote("ASSET_A", dec!(100), date, "USD"); // $100 * 100 = $10K
    let quote_b = create_test_quote("ASSET_B", dec!(100), date, "USD"); // $100 * 50 = $5K
    let quote_c = create_test_quote("ASSET_C", dec!(100), date, "USD"); // $100 * 150 = $15K (excluded)
    let quote_d = create_test_quote("ASSET_D", dec!(100), date, "USD"); // $100 * 200 = $20K (excluded)

    let service = create_net_worth_service(
        vec![acc_a, acc_b, acc_c, acc_d],
        vec![asset_a, asset_b, asset_c, asset_d],
        vec![snap_a, snap_b, snap_c, snap_d],
        vec![quote_a, quote_b, quote_c, quote_d],
    );

    let result = service.get_net_worth(date).await.unwrap();

    // Only non-archived accounts (A and B) should be included
    // A: $10K + B: $5K = $15K total
    assert_eq!(result.net_worth, dec!(15000));
    assert_eq!(result.assets.total, dec!(15000));
}

#[tokio::test]
async fn test_all_accounts_archived_returns_zero_net_worth() {
    // All accounts are archived - net worth should be zero
    let acc1 = create_test_account_with_archive_state("archived1", "SECURITIES", "USD", true, true);
    let acc2 =
        create_test_account_with_archive_state("archived2", "SECURITIES", "USD", false, true);

    let asset1 = create_test_asset("ASSET_1", AssetKind::Investment, "USD");
    let asset2 = create_test_asset("ASSET_2", AssetKind::Investment, "USD");

    let pos1 = create_test_position("archived1", "ASSET_1", dec!(100), dec!(10000), "USD");
    let pos2 = create_test_position("archived2", "ASSET_2", dec!(200), dec!(20000), "USD");

    let snap1 = create_test_snapshot("archived1", vec![pos1], HashMap::new());
    let snap2 = create_test_snapshot("archived2", vec![pos2], HashMap::new());

    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote1 = create_test_quote("ASSET_1", dec!(150), date, "USD");
    let quote2 = create_test_quote("ASSET_2", dec!(150), date, "USD");

    let service = create_net_worth_service(
        vec![acc1, acc2],
        vec![asset1, asset2],
        vec![snap1, snap2],
        vec![quote1, quote2],
    );

    let result = service.get_net_worth(date).await.unwrap();

    // All accounts are archived - should return zero
    assert_eq!(result.net_worth, Decimal::ZERO);
    assert_eq!(result.assets.total, Decimal::ZERO);
}

#[tokio::test]
async fn test_newly_created_account_has_default_archive_values_net_worth() {
    // Verify default values for newly created accounts
    let account = Account::default();
    assert!(
        !account.is_archived,
        "New accounts should not be archived by default"
    );

    let account = create_test_account("new_acc", "SECURITIES", "USD");
    assert!(
        !account.is_archived,
        "Test helper should create non-archived accounts"
    );
    assert!(
        account.is_active,
        "Test helper should create active accounts"
    );
}

// ============================================================================
// Verified Receipt Eligibility Tests
// ============================================================================

#[tokio::test]
async fn test_net_worth_excludes_account_without_verified_receipt() {
    let account =
        create_test_account_with_archive_state("no_receipt", "SECURITIES", "USD", true, false);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("no_receipt", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("no_receipt", vec![position], HashMap::new());
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(185), date, "USD");

    let policy = Arc::new(MockAggregateEligibilityService::with_rows(&[], &[], &[]));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![account]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![snapshot]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();
    assert_eq!(
        result.net_worth,
        Decimal::ZERO,
        "missing receipt => account contributes nothing"
    );
    assert_eq!(result.assets.total, Decimal::ZERO);
}

#[tokio::test]
async fn test_net_worth_includes_account_with_verified_receipt() {
    let account =
        create_test_account_with_archive_state("has_receipt", "SECURITIES", "USD", true, false);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let position = create_test_position("has_receipt", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("has_receipt", vec![position], HashMap::new());
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(185), date, "USD");

    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["has_receipt"],
        &[snapshot.id.as_str()],
        &[],
    ));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![account]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![snapshot]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();
    assert_eq!(
        result.net_worth,
        dec!(18500),
        "verified receipt => account contributes normally"
    );
    assert_eq!(result.assets.total, dec!(18500));
}

#[tokio::test]
async fn test_net_worth_mixed_eligibility() {
    let with_receipt =
        create_test_account_with_archive_state("with_receipt", "SECURITIES", "USD", true, false);
    let without_receipt =
        create_test_account_with_archive_state("without_receipt", "SECURITIES", "USD", true, false);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let pos_w = create_test_position("with_receipt", "AAPL", dec!(50), dec!(5000), "USD");
    let pos_wo = create_test_position("without_receipt", "AAPL", dec!(100), dec!(10000), "USD");
    let snap_w = create_test_snapshot("with_receipt", vec![pos_w], HashMap::new());
    let snap_wo = create_test_snapshot("without_receipt", vec![pos_wo], HashMap::new());
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(200), date, "USD");

    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["with_receipt"],
        &[snap_w.id.as_str()],
        &[],
    ));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![
        with_receipt,
        without_receipt,
    ]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![snap_w, snap_wo]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();
    assert_eq!(
        result.net_worth,
        dec!(10000),
        "only receipt-holder contributes"
    );
    assert_eq!(result.assets.total, dec!(10000));
}

// ============================================================================
// Regression Tests
// ============================================================================

/// Regression: a liability that has a future-dated $0 quote (e.g. a mortgage planned
/// payoff date) must NOT appear as fully paid-off when computing net worth today.
/// The `get_latest_quote_as_of` logic filters quotes to those on or before the
/// requested date, so the future $0 row should be ignored and the most-recent
/// past balance ($180,000) should be used instead.
#[tokio::test]
async fn test_liability_with_future_zero_quote_is_included_at_past_balance() {
    let valuation_date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    // A mortgage LIABILITY account and asset
    let liab_account = create_test_account("liab-mortgage", "LIABILITY", "USD");
    let liab_asset = create_test_asset("LIAB-mortgage-reg", AssetKind::Liability, "USD");

    // The position records the outstanding balance as quantity=1, cost-basis=180_000
    let liab_position = create_test_position(
        "liab-mortgage",
        "LIAB-mortgage-reg",
        dec!(1),
        dec!(180000),
        "USD",
    );
    let liab_snapshot = create_test_snapshot("liab-mortgage", vec![liab_position], HashMap::new());

    // Two quotes for the same liability:
    //   • 2024-01-10: current outstanding balance = $180,000
    //   • 2041-01-01: planned payoff date recorded as $0 (future-dated)
    let past_quote = create_test_quote(
        "LIAB-mortgage-reg",
        dec!(180000),
        NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        "USD",
    );
    let future_zero_quote = create_test_quote(
        "LIAB-mortgage-reg",
        dec!(0),
        NaiveDate::from_ymd_opt(2041, 1, 1).unwrap(),
        "USD",
    );

    let service = create_net_worth_service(
        vec![liab_account],
        vec![liab_asset],
        vec![liab_snapshot],
        vec![past_quote, future_zero_quote],
    );

    let result = service.get_net_worth(valuation_date).await.unwrap();

    // The future $0 quote must be excluded; the liability must appear at $180,000.
    assert_eq!(
        result.liabilities.total,
        dec!(180000),
        "liability with a future-dated $0 quote should be valued at the most-recent past quote"
    );
    // Net worth = assets (0) - liabilities (180_000) = -180_000
    assert_eq!(result.net_worth, dec!(-180000));
}

#[tokio::test]
async fn test_net_worth_history_excludes_account_without_verified_receipt() {
    let account_no_receipt =
        create_test_account_with_archive_state("hist-no-receipt", "SECURITIES", "USD", true, false);
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let pos = create_test_position("hist-no-receipt", "AAPL", dec!(100), dec!(15000), "USD");
    let snapshot = create_test_snapshot("hist-no-receipt", vec![pos], HashMap::new());
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(185), date, "USD");

    let policy = Arc::new(MockAggregateEligibilityService::with_rows(&[], &[], &[]));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![account_no_receipt]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![snapshot]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let history = service
        .get_net_worth_history(date, date)
        .await
        .expect("history query");
    assert!(
        history.is_empty() || history.iter().all(|p| p.net_worth == Decimal::ZERO),
        "account without verified receipt must contribute nothing to history"
    );
}

#[tokio::test]
async fn test_net_worth_history_includes_account_with_verified_receipt() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();
    let d3 = NaiveDate::from_ymd_opt(2024, 1, 17).unwrap();

    let account = create_test_account_with_archive_state(
        "hist-has-receipt",
        "SECURITIES",
        "USD",
        true,
        false,
    );
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let quote = create_test_quote("AAPL", dec!(185), d1, "USD");

    let snap = create_test_snapshot(
        "hist-has-receipt",
        vec![create_test_position(
            "hist-has-receipt",
            "AAPL",
            dec!(100),
            dec!(15000),
            "USD",
        )],
        HashMap::new(),
    );

    // Use the same DailyAccountValuation pattern as other history tests
    fn mk_val(account_id: &str, date: NaiveDate, tv: Decimal) -> DailyAccountValuation {
        DailyAccountValuation {
            id: format!("{}_{}", account_id, date),
            account_id: account_id.to_string(),
            valuation_date: date,
            account_currency: "USD".to_string(),
            base_currency: "USD".to_string(),
            fx_rate_to_base: dec!(1),
            cash_balance: Decimal::ZERO,
            investment_market_value: tv,
            total_value: tv,
            cost_basis: dec!(15000),
            book_basis: dec!(15000),
            net_contribution: dec!(15000),
            cash_balance_base: Decimal::ZERO,
            investment_market_value_base: tv,
            total_value_base: tv,
            cost_basis_base: dec!(15000),
            book_basis_base: dec!(15000),
            net_contribution_base: dec!(15000),
            external_inflow_base: Decimal::ZERO,
            external_outflow_base: Decimal::ZERO,
            external_flow_source: ExternalFlowSource::NoFlow,
            performance_eligible_value_base: tv,
            value_status: ValuationStatus::Complete,
            basis_status: BasisStatus::Complete,
            calculated_at: DateTime::<Utc>::from_timestamp(1_776_480_000, 0).unwrap(),
        }
    }

    let valuations = vec![
        mk_val("hist-has-receipt", d1, dec!(18500)),
        mk_val("hist-has-receipt", d2, dec!(18600)),
        mk_val("hist-has-receipt", d3, dec!(18700)),
    ];

    let eligible_valuation_ids: Vec<&str> = valuations
        .iter()
        .map(|valuation| valuation.id.as_str())
        .collect();
    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["hist-has-receipt"],
        &[snap.id.as_str()],
        &eligible_valuation_ids,
    ));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![account]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![snap]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(valuations));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let history = service
        .get_net_worth_history(d1, d3)
        .await
        .expect("history query");
    assert_eq!(
        history.len(),
        3,
        "account with verified receipt must have 3 history points"
    );
    assert!(
        history.iter().all(|p| p.net_worth > Decimal::ZERO),
        "all points must have positive net worth"
    );
}

#[tokio::test]
async fn test_net_worth_history_batches_row_policy_and_filters_semantic_roles() {
    let d0 = NaiveDate::from_ymd_opt(2024, 1, 14).unwrap();
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let inv_live = create_test_account_with_archive_state(
        "hist-live",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let inv_context = create_test_account_with_archive_state(
        "hist-context",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let card = create_test_account_with_archive_state(
        "hist-card",
        account_types::CREDIT_CARD,
        "USD",
        true,
        false,
    );

    fn credit_card_valuation(
        account_id: &str,
        date: NaiveDate,
        liability: Decimal,
    ) -> DailyAccountValuation {
        let mut valuation = create_account_valuation(account_id, date, -liability);
        valuation.cash_balance = -liability;
        valuation.cash_balance_base = -liability;
        valuation.total_value = -liability;
        valuation.total_value_base = -liability;
        valuation
    }

    let valuations = vec![
        create_account_valuation("hist-live", d1, dec!(100)),
        create_account_valuation("hist-live", d2, dec!(200)),
        create_account_valuation("hist-context", d1, dec!(1000)),
        credit_card_valuation("hist-card", d0, dec!(20)),
        credit_card_valuation("hist-card", d1, dec!(30)),
        credit_card_valuation("hist-card", d2, dec!(40)),
    ];

    let account_records = HashMap::from([
        ("hist-live".to_string(), SemanticPolicyRecord::live()),
        (
            "hist-context".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
        ("hist-card".to_string(), SemanticPolicyRecord::live()),
    ]);
    let row_records = HashMap::from([
        (
            "hist-live_2024-01-15".to_string(),
            SemanticPolicyRecord::live(),
        ),
        (
            "hist-live_2024-01-16".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
        (
            "hist-card_2024-01-14".to_string(),
            SemanticPolicyRecord::live(),
        ),
        (
            "hist-card_2024-01-15".to_string(),
            SemanticPolicyRecord::live(),
        ),
        (
            "hist-card_2024-01-16".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
    ]);
    let policy = Arc::new(RecordingSemanticEligibilityService::with_records(
        account_records,
        row_records,
    ));
    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![
            inv_live,
            inv_context,
            card,
        ])),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(valuations)),
        Arc::new(MockFxService::new("USD")),
        policy.clone(),
    );

    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, d1);
    assert_eq!(history[0].net_worth, dec!(70));

    let mut account_calls = policy.account_calls.lock().unwrap().clone();
    for (account_ids, _) in &mut account_calls {
        account_ids.sort();
    }
    assert_eq!(
        account_calls,
        vec![(
            vec![
                "hist-card".to_string(),
                "hist-context".to_string(),
                "hist-live".to_string(),
            ],
            d2,
        )]
    );
    assert_eq!(
        *policy.row_calls.lock().unwrap(),
        vec![(
            ENTITY_DAILY_VALUATION.to_string(),
            vec![
                ("hist-live_2024-01-15".to_string(), d1),
                ("hist-live_2024-01-16".to_string(), d2),
                ("hist-context_2024-01-15".to_string(), d1),
                ("hist-card_2024-01-14".to_string(), d0),
                ("hist-card_2024-01-15".to_string(), d1),
                ("hist-card_2024-01-16".to_string(), d2),
            ],
        )]
    );
}

#[tokio::test]
async fn net_worth_history_exact_account_date_pairs_filter_before_aggregation() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2024, 1, 16).unwrap();

    let live_account = create_test_account_with_archive_state(
        "pair-live",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let contextual_account = create_test_account_with_archive_state(
        "pair-context",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let valuations = vec![
        create_account_valuation("pair-live", d1, dec!(100)),
        create_account_valuation("pair-live", d2, dec!(200)),
        create_account_valuation("pair-context", d1, dec!(1000)),
    ];
    let account_records = HashMap::from([
        ("pair-live".to_string(), SemanticPolicyRecord::live()),
        (
            "pair-context".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
    ]);
    let row_records = HashMap::from([
        (
            "pair-live_2024-01-15".to_string(),
            SemanticPolicyRecord::live(),
        ),
        (
            "pair-live_2024-01-16".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
        (
            "pair-context_2024-01-15".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
    ]);
    let policy = Arc::new(RecordingSemanticEligibilityService::with_records(
        account_records,
        row_records,
    ));
    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![
            live_account,
            contextual_account,
        ])),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(valuations)),
        Arc::new(MockFxService::new("USD")),
        policy.clone(),
    );

    let history = service.get_net_worth_history(d1, d2).await.unwrap();

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, d1);
    assert_eq!(history[0].net_worth, dec!(100));

    let mut account_calls = policy.account_calls.lock().unwrap().clone();
    for (account_ids, _) in &mut account_calls {
        account_ids.sort();
    }
    assert_eq!(
        account_calls,
        vec![(
            vec!["pair-context".to_string(), "pair-live".to_string()],
            d2
        )]
    );
    assert_eq!(
        *policy.row_calls.lock().unwrap(),
        vec![(
            ENTITY_DAILY_VALUATION.to_string(),
            vec![
                ("pair-live_2024-01-15".to_string(), d1),
                ("pair-live_2024-01-16".to_string(), d2),
                ("pair-context_2024-01-15".to_string(), d1),
            ],
        )]
    );
}

#[tokio::test]
async fn net_worth_account_policy_error_propagates_without_partial_result() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let account = create_test_account_with_archive_state(
        "account-policy",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let valuation = create_account_valuation("account-policy", d1, dec!(100));
    let mut policy = SemanticEligibilityServiceWithErrors::with_records(
        HashMap::from([("account-policy".to_string(), SemanticPolicyRecord::live())]),
        HashMap::from([(
            "account-policy_2024-01-15".to_string(),
            SemanticPolicyRecord::live(),
        )]),
    );
    policy.account_error = Some("account-policy".to_string());
    let policy = Arc::new(policy);
    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![account])),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(vec![valuation])),
        Arc::new(MockFxService::new("USD")),
        policy.clone(),
    );

    let error = service.get_net_worth_history(d1, d1).await.unwrap_err();

    assert!(matches!(
        error,
        crate::Error::ConstraintViolation(message) if message == "account-policy"
    ));
    assert_eq!(
        *policy.account_calls.lock().unwrap(),
        vec![(vec!["account-policy".to_string()], d1)]
    );
    assert!(policy.row_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn net_worth_row_policy_error_propagates_without_partial_history() {
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let account = create_test_account_with_archive_state(
        "row-policy",
        account_types::SECURITIES,
        "USD",
        true,
        false,
    );
    let valuation = create_account_valuation("row-policy", d1, dec!(100));
    let mut policy = SemanticEligibilityServiceWithErrors::with_records(
        HashMap::from([(String::from("row-policy"), SemanticPolicyRecord::live())]),
        HashMap::from([(
            String::from("row-policy_2024-01-15"),
            SemanticPolicyRecord::live(),
        )]),
    );
    policy.row_error = Some("row-policy".to_string());
    let policy = Arc::new(policy);
    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(vec![account])),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(vec![valuation])),
        Arc::new(MockFxService::new("USD")),
        policy.clone(),
    );

    let error = service.get_net_worth_history(d1, d1).await.unwrap_err();

    assert!(matches!(
        error,
        crate::Error::ConstraintViolation(message) if message == "row-policy"
    ));
    assert_eq!(
        *policy.account_calls.lock().unwrap(),
        vec![(vec!["row-policy".to_string()], d1)]
    );
    assert_eq!(
        *policy.row_calls.lock().unwrap(),
        vec![(
            ENTITY_DAILY_VALUATION.to_string(),
            vec![("row-policy_2024-01-15".to_string(), d1)],
        )]
    );
}

#[tokio::test]
async fn net_worth_semantic_public_path_matrix_fails_closed_by_role_and_date() {
    let before = NaiveDate::from_ymd_opt(2024, 1, 14).unwrap();
    let covered = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let account_ids = [
        "live",
        "baseline",
        "manual-locked",
        "manual-open",
        "missing",
        "unknown",
        "context",
        "reference",
    ];
    let accounts = account_ids
        .iter()
        .map(|id| {
            create_test_account_with_archive_state(
                id,
                account_types::SECURITIES,
                "USD",
                true,
                false,
            )
        })
        .collect::<Vec<_>>();
    let valuations = account_ids
        .iter()
        .map(|id| create_account_valuation(id, covered, dec!(10)))
        .collect::<Vec<_>>();
    let account_records = HashMap::from([
        ("live".to_string(), SemanticPolicyRecord::live()),
        (
            "baseline".to_string(),
            SemanticPolicyRecord {
                role: ProvenanceRole::OpeningBaseline,
                coverage_start_date: Some(covered),
                manual_current_approved: false,
            },
        ),
        (
            "manual-locked".to_string(),
            SemanticPolicyRecord {
                role: ProvenanceRole::ManualCurrent,
                coverage_start_date: None,
                manual_current_approved: false,
            },
        ),
        (
            "manual-open".to_string(),
            SemanticPolicyRecord {
                role: ProvenanceRole::ManualCurrent,
                coverage_start_date: None,
                manual_current_approved: true,
            },
        ),
        (
            "context".to_string(),
            SemanticPolicyRecord::historical_context(),
        ),
        (
            "reference".to_string(),
            SemanticPolicyRecord {
                role: ProvenanceRole::ContextualReference,
                coverage_start_date: None,
                manual_current_approved: false,
            },
        ),
    ]);
    let row_records = valuations
        .iter()
        .map(|v| {
            (
                v.id.clone(),
                account_records
                    .get(&v.account_id)
                    .cloned()
                    .unwrap_or_else(SemanticPolicyRecord::historical_context),
            )
        })
        .collect();
    let policy = Arc::new(RecordingSemanticEligibilityService::with_records(
        account_records,
        row_records,
    ));
    let service = NetWorthService::new(
        Arc::new(RwLock::new("USD".to_string())),
        Arc::new(MockAccountRepository::new(accounts)),
        Arc::new(MockAssetRepository::new(vec![])),
        Arc::new(MockSnapshotRepository::new(vec![])),
        Arc::new(MockMarketDataRepository::new(vec![])),
        Arc::new(MockValuationRepository::new(valuations)),
        Arc::new(MockFxService::new("USD")),
        policy,
    );
    let history = service
        .get_net_worth_history(before, covered)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].date, covered);
    assert_eq!(history[0].net_worth, dec!(30));
}

#[tokio::test]
async fn test_net_worth_mixed_receipt_status_isolation() {
    // Two accounts, both include_in_net_worth=true, but only one has a verified receipt.
    // Only the receipted account should contribute to net worth.
    let receipt_acct =
        create_test_account_with_archive_state("receipt-acct", "SECURITIES", "USD", true, false);
    let no_receipt_acct =
        create_test_account_with_archive_state("no-receipt-acct", "SECURITIES", "USD", true, false);

    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    let receipt_position =
        create_test_position("receipt-acct", "AAPL", dec!(50), dec!(10000), "USD");
    let no_receipt_position =
        create_test_position("no-receipt-acct", "AAPL", dec!(100), dec!(20000), "USD");

    let receipt_snapshot =
        create_test_snapshot("receipt-acct", vec![receipt_position], HashMap::new());
    let no_receipt_snapshot =
        create_test_snapshot("no-receipt-acct", vec![no_receipt_position], HashMap::new());

    let quote = create_test_quote("AAPL", dec!(200), date, "USD");

    // Only grant eligibility to receipt-acct
    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["receipt-acct"],
        &[receipt_snapshot.id.as_str()],
        &[],
    ));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![
        receipt_acct,
        no_receipt_acct,
    ]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![
        receipt_snapshot,
        no_receipt_snapshot,
    ]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();

    // Only the receipted account's value (50 shares × $200 = $10,000) contributes
    assert_eq!(
        result.net_worth,
        dec!(10000),
        "only receipted account value contributes; non-receipted account must be excluded"
    );
    assert_eq!(
        result
            .assets
            .breakdown
            .iter()
            .map(|b| b.value)
            .sum::<Decimal>(),
        dec!(10000),
        "assets breakdown must only reflect the receipted account"
    );
}
// Regression: include_in_net_worth=false combined with receipt eligibility
#[tokio::test]
async fn test_net_worth_opted_out_with_receipt_still_excluded() {
    // Even if an account has a receipt, if include_in_net_worth=false
    // it must still be excluded. The get_eligible_account_ids() SQL query
    // already filters for include_in_net_worth=1, so the account won't
    // appear in the eligible_ids list. But this regression verifies the
    // double-protection: the initial include_in_net_worth filter in
    // get_net_worth() catches it before the receipt check.
    let excluded = Account {
        include_in_net_worth: false,
        name_source: "provider".to_string(),
        ..create_test_account("opted-out-receipt", "SECURITIES", "USD")
    };
    let included = create_test_account("included-account", "SECURITIES", "USD");
    let asset = create_test_asset("AAPL", AssetKind::Investment, "USD");
    let excluded_pos =
        create_test_position("opted-out-receipt", "AAPL", dec!(50), dec!(5000), "USD");
    let included_pos =
        create_test_position("included-account", "AAPL", dec!(10), dec!(1000), "USD");
    let excluded_snap =
        create_test_snapshot("opted-out-receipt", vec![excluded_pos], HashMap::new());
    let included_snap =
        create_test_snapshot("included-account", vec![included_pos], HashMap::new());
    let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
    let quote = create_test_quote("AAPL", dec!(200), date, "USD");

    // Give BOTH accounts eligible IDs to test that the initial
    // include_in_net_worth gate catches the excluded one
    let policy = Arc::new(MockAggregateEligibilityService::with_rows(
        &["opted-out-receipt", "included-account"],
        &[excluded_snap.id.as_str(), included_snap.id.as_str()],
        &[],
    ));
    let base_currency = Arc::new(RwLock::new("USD".to_string()));
    let account_repo = Arc::new(MockAccountRepository::new(vec![excluded, included]));
    let asset_repo = Arc::new(MockAssetRepository::new(vec![asset]));
    let snapshot_repo = Arc::new(MockSnapshotRepository::new(vec![
        excluded_snap,
        included_snap,
    ]));
    let market_data_repo = Arc::new(MockMarketDataRepository::new(vec![quote]));
    let valuation_repo = Arc::new(MockValuationRepository::new(vec![]));
    let fx_service = Arc::new(MockFxService::new("USD"));

    let service = NetWorthService::new(
        base_currency,
        account_repo,
        asset_repo,
        snapshot_repo,
        market_data_repo,
        valuation_repo,
        fx_service,
        policy,
    );

    let result = service.get_net_worth(date).await.unwrap();
    // Only the include_in_net_worth=true account contributes
    assert_eq!(
        result.net_worth,
        dec!(2000),
        "opted-out with receipt must still be excluded"
    );
    assert_eq!(result.assets.total, dec!(2000));
}
