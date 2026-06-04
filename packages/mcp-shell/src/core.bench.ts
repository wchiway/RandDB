import { bench, describe } from 'vitest';
import { formatCoreResult } from './core.js';
import type { CoreResult } from './contracts.js';
import { buildCoreRequest } from './tools.js';

const payload = {
  repo_path: '/repo',
  information_request: 'Find retrieval flow',
  technical_terms: ['SearchService'],
  mode: 'balanced' as const,
  exclude_globs: ['target/**'],
  language: ['rust'],
  max_total_chars: 20_000,
  max_files: 5,
  max_segments_per_file: 2,
  return_debug: true,
  low_confidence_behavior: 'return_with_warning' as const,
  output_format: 'both' as const,
};

const retrievalResult: CoreResult = {
  status: 'ok',
  response: {
    operation: 'codebase-retrieval',
    payload: {
      format: 'both',
      markdown: 'Found 1 relevant code block | Files: 1 | Total segments: 1\n',
      pack: {
        files: [
          {
            path: 'src/lib.rs',
            language: 'rust',
            segments: [
              {
                start_line: 1,
                end_line: 2,
                start_utf16: 0,
                end_utf16: 42,
                breadcrumb: 'src/lib.rs',
                text: 'pub fn source() {}\nfn caller() { source(); }\n',
                score: 0.9,
              },
            ],
          },
        ],
        char_count: 42,
        truncated: false,
      },
      debug: {
        lexical_query: 'source',
        semantic_query: 'Find source function',
        candidate_count: 1,
        packed_segment_count: 1,
      },
      warnings: [],
    },
  },
};

describe('MCP shell adapter overhead', () => {
  bench('builds stable core JSON requests', () => {
    JSON.stringify(buildCoreRequest('codebase-retrieval', payload));
  });

  bench('formats retrieval CoreResult as MCP text content', () => {
    formatCoreResult(retrievalResult);
  });
});
