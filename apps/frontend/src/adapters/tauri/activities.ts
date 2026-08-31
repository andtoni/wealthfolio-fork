// Tauri-specific activity commands
import type {
  InvestecCardLifecycleLinkRequest,
  InvestecCardLifecycleLinkResponse,
  ParseConfig,
  ParsedCsvResult,
} from "@/lib/types";
import { invoke, logger } from "./core";

export const linkInvestecCardLifecycle = async (
  _request: InvestecCardLifecycleLinkRequest,
): Promise<InvestecCardLifecycleLinkResponse> => {
  throw new Error("Investec card lifecycle linking is available only in enabled web deployments.");
};

/**
 * Parse a CSV file with the given configuration.
 * Tauri implementation: reads file as ArrayBuffer and invokes parse_csv command.
 */
export const parseCsv = async (file: File, config: ParseConfig): Promise<ParsedCsvResult> => {
  try {
    const buffer = await file.arrayBuffer();
    const content = Array.from(new Uint8Array(buffer));
    return await invoke<ParsedCsvResult>("parse_csv", { content, config });
  } catch (err) {
    logger.error("Error parsing CSV file:", err);
    throw err;
  }
};
