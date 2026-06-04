import { bench, describe } from 'vitest';
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

describe('MCP shell adapter overhead', () => {
  bench('builds stable core JSON requests', () => {
    JSON.stringify(buildCoreRequest('codebase-retrieval', payload));
  });
});
