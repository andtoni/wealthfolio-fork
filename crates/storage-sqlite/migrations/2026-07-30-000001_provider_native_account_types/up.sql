-- Connected provider account types must use Wealthfolio's native account model.
-- Investec and Mercury accounts are signed cash ledgers, including legitimate
-- negative Investec balances. Plaid liabilities use the available native
-- liability type; supported investment subtypes remain securities.
UPDATE accounts
SET account_type = CASE
    WHEN lower(provider) = 'plaid'
      AND lower(account_type) IN ('credit card', 'line of credit', 'loan', 'mortgage', 'student loan')
      THEN 'CREDIT_CARD'
    WHEN lower(provider) = 'plaid'
      AND lower(account_type) IN (
        '401a', '401k', '403b', '457b', 'brokerage', 'hsa', 'ira', 'keogh',
        'lira', 'pension', 'roth', 'rrif', 'rrsp', 'sep ira', 'simple ira',
        'sipp', 'stock plan', 'tfsa', 'trust', 'ugma', 'utma'
      )
      THEN 'SECURITIES'
    ELSE 'CASH'
  END,
  updated_at = CURRENT_TIMESTAMP
WHERE provider_account_id IS NOT NULL
  AND lower(provider) IN ('investec', 'mercury', 'plaid')
  AND account_type NOT IN ('CASH', 'CREDIT_CARD', 'SECURITIES');
