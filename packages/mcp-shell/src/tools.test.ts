import { describe, expect, it } from 'vitest';
import { buildCoreRequest, codebaseRetrievalSchema, findReferencesSchema, toolDefinitions } from './tools.js';

describe('tool definitions', () => {
  it('registers the ContextWeaver-compatible tool surface', () => {
    expect(toolDefinitions.map((tool) => tool.name)).toEqual([
      'codebase-retrieval',
      'list-files',
      'find-references',
      'get-symbol-definition',
      'stats',
    ]);
  });

  it('builds codebase-retrieval core requests without changing payload names', () => {
    const payload = codebaseRetrievalSchema.parse({
      repo_path: '/repo',
      information_request: 'Find retrieval flow',
      technical_terms: ['SearchService'],
      mode: 'balanced',
      exclude_globs: ['target/**'],
      language: ['rust'],
      max_total_chars: 20000,
      max_files: 5,
      max_segments_per_file: 2,
      return_debug: true,
      low_confidence_behavior: 'return_with_warning',
      output_format: 'both',
    });

    expect(buildCoreRequest('codebase-retrieval', payload)).toEqual({
      operation: 'codebase-retrieval',
      payload,
    });
  });

  it('validates reference tool bounds before invoking Rust', () => {
    expect(() =>
      findReferencesSchema.parse({ repo_path: '/repo', symbol: 'Thing', max_results: 201 }),
    ).toThrow();
  });
});
