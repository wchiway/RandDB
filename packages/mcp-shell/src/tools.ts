import * as z from 'zod/v4';
import { invokeCoreTool, type CoreInvokerOptions } from './core.js';
import type {
  CodebaseRetrievalInput,
  CoreRequest,
  FindReferencesInput,
  GetSymbolDefinitionInput,
  ListFilesInput,
  StatsInput,
  TextToolResponse,
} from './contracts.js';

const positiveInt = z.number().int().positive();

export const codebaseRetrievalSchema = z.object({
  repo_path: z.string(),
  information_request: z.string(),
  technical_terms: z.array(z.string()).optional(),
  mode: z.enum(['quick', 'balanced', 'deep']).optional(),
  include_globs: z.array(z.string()).optional(),
  exclude_globs: z.array(z.string()).optional(),
  language: z.array(z.string()).optional(),
  max_total_chars: positiveInt.optional(),
  max_files: positiveInt.optional(),
  max_segments_per_file: positiveInt.optional(),
  return_debug: z.boolean().optional(),
  low_confidence_behavior: z
    .enum(['return_top1', 'return_empty', 'return_with_warning'])
    .optional(),
  output_format: z.enum(['markdown', 'json', 'both']).optional(),
});

export const listFilesSchema = z.object({
  repo_path: z.string(),
  glob: z.string().optional(),
  language: z.string().optional(),
  max_results: positiveInt.optional(),
});

export const findReferencesSchema = z.object({
  repo_path: z.string(),
  symbol: z.string().min(1),
  exclude_definition: z.boolean().optional(),
  max_results: positiveInt.max(200).optional(),
});

export const getSymbolDefinitionSchema = z.object({
  repo_path: z.string(),
  symbol: z.string().min(1),
  hint_path: z.string().optional(),
  max_results: positiveInt.max(20).optional(),
});

export const statsSchema = z.object({
  repo_path: z.string(),
});

export type ToolName =
  | 'codebase-retrieval'
  | 'list-files'
  | 'find-references'
  | 'get-symbol-definition'
  | 'stats';

export interface ToolDefinition {
  name: ToolName;
  description: string;
  inputSchema: z.ZodType<unknown>;
}

export const toolDefinitions = [
  {
    name: 'codebase-retrieval',
    description: 'Primary hybrid semantic plus exact-match retrieval tool.',
    inputSchema: codebaseRetrievalSchema,
  },
  {
    name: 'list-files',
    description: 'List indexed file metadata without embedding cost.',
    inputSchema: listFilesSchema,
  },
  {
    name: 'find-references',
    description: 'Find heuristic text references to a known symbol.',
    inputSchema: findReferencesSchema,
  },
  {
    name: 'get-symbol-definition',
    description: 'Find likely symbol definition blocks for a known symbol.',
    inputSchema: getSymbolDefinitionSchema,
  },
  {
    name: 'stats',
    description: 'Show RandDB index, search, and health statistics.',
    inputSchema: statsSchema,
  },
] as const satisfies readonly ToolDefinition[];

export function buildCoreRequest(name: ToolName, rawInput: unknown): CoreRequest {
  switch (name) {
    case 'codebase-retrieval':
      return { operation: name, payload: codebaseRetrievalSchema.parse(rawInput) };
    case 'list-files':
      return { operation: name, payload: listFilesSchema.parse(rawInput) };
    case 'find-references':
      return { operation: name, payload: findReferencesSchema.parse(rawInput) };
    case 'get-symbol-definition':
      return { operation: name, payload: getSymbolDefinitionSchema.parse(rawInput) };
    case 'stats':
      return { operation: name, payload: statsSchema.parse(rawInput) };
  }
}

export async function callTool(
  name: ToolName,
  rawInput: unknown,
  options?: CoreInvokerOptions,
): Promise<TextToolResponse> {
  return invokeCoreTool(buildCoreRequest(name, rawInput), options);
}
