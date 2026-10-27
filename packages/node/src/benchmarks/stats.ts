export interface BenchmarkResult {
  surface: string;
  codec: "cleartext" | "aes-256-gcm";
  workload: number;
  batchSize: number;
  totalDurationMs: number;
  throughputOpsSec: number;
  p50Us: number;
  p95Us: number;
  p99Us: number;
  meanUs: number;
  minUs: number;
  maxUs: number;
  stddevUs: number;
  diskBytes?: number;
}

export function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return 0;
  if (sorted.length === 1) return sorted[0]!;
  const rank = (p / 100) * (sorted.length - 1);
  const lower = Math.floor(rank);
  const upper = Math.ceil(rank);
  const weight = rank - lower;
  const valLower = sorted[lower]!;
  const valUpper = sorted[upper]!;
  return valLower * (1 - weight) + valUpper * weight;
}

export function calculateStats(
  latenciesUs: number[],
  totalMutations: number,
  totalDurationMs: number,
  meta: {
    surface: string;
    codec: "cleartext" | "aes-256-gcm";
    workload: number;
    batchSize: number;
    diskBytes?: number;
  }
): BenchmarkResult {
  const sorted = [...latenciesUs].sort((a, b) => a - b);
  const p50Us = percentile(sorted, 50);
  const p95Us = percentile(sorted, 95);
  const p99Us = percentile(sorted, 99);
  const minUs = sorted[0] ?? 0;
  const maxUs = sorted[sorted.length - 1] ?? 0;

  const sum = sorted.reduce((acc, v) => acc + v, 0);
  const meanUs = sorted.length > 0 ? sum / sorted.length : 0;

  const variance =
    sorted.length > 0
      ? sorted.reduce((acc, v) => acc + Math.pow(v - meanUs, 2), 0) / sorted.length
      : 0;
  const stddevUs = Math.sqrt(variance);

  const throughputOpsSec =
    totalDurationMs > 0 ? (totalMutations / totalDurationMs) * 1000 : 0;

  return {
    surface: meta.surface,
    codec: meta.codec,
    workload: meta.workload,
    batchSize: meta.batchSize,
    totalDurationMs,
    throughputOpsSec,
    p50Us,
    p95Us,
    p99Us,
    meanUs,
    minUs,
    maxUs,
    stddevUs,
    diskBytes: meta.diskBytes,
  };
}
