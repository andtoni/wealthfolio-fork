ALTER TABLE holdings_snapshots
    ADD COLUMN position_market_values TEXT NOT NULL DEFAULT '{}'
    CHECK(json_valid(position_market_values));
ALTER TABLE holdings_snapshots
    ADD COLUMN liability_balance TEXT NOT NULL DEFAULT '0';
