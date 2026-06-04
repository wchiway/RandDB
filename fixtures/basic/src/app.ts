export class SearchTarget {
  constructor(private readonly name: string) {}

  describe(): string {
    return `target:${this.name}`;
  }
}

export function rankScore(exactMatches: number, semanticScore: number): number {
  return exactMatches + semanticScore;
}
