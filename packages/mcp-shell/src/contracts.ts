export type RetrievalMode = 'quick' | 'balanced' | 'deep';
export type LowConfidenceBehavior = 'return_top1' | 'return_empty' | 'return_with_warning';
export type OutputFormat = 'markdown' | 'json' | 'both';

export type CoreRequest =
  | { operation: 'codebase-retrieval'; payload: CodebaseRetrievalInput }
  | { operation: 'list-files'; payload: ListFilesInput }
  | { operation: 'find-references'; payload: FindReferencesInput }
  | { operation: 'get-symbol-definition'; payload: GetSymbolDefinitionInput }
  | { operation: 'stats'; payload: StatsInput };

export interface CodebaseRetrievalInput {
  repo_path: string;
  information_request: string;
  technical_terms?: string[];
  mode?: RetrievalMode;
  include_globs?: string[];
  exclude_globs?: string[];
  language?: string[];
  max_total_chars?: number;
  max_files?: number;
  max_segments_per_file?: number;
  return_debug?: boolean;
  low_confidence_behavior?: LowConfidenceBehavior;
  output_format?: OutputFormat;
}

export interface ListFilesInput {
  repo_path: string;
  glob?: string;
  language?: string;
  max_results?: number;
}

export interface FindReferencesInput {
  repo_path: string;
  symbol: string;
  exclude_definition?: boolean;
  max_results?: number;
}

export interface GetSymbolDefinitionInput {
  repo_path: string;
  symbol: string;
  hint_path?: string;
  max_results?: number;
}

export interface StatsInput {
  repo_path: string;
}

export interface CoreResult {
  status: 'ok' | 'error';
  response?: unknown;
  error?: {
    code: string;
    message: string;
    operation?: string;
    retryable: boolean;
  };
}

export interface TextToolResponse {
  [key: string]: unknown;
  content: Array<{ type: 'text'; text: string }>;
  isError?: boolean;
}
