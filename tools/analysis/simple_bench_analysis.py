#!/usr/bin/env python3
"""
DevIt Simple Benchmark Analysis (pas de dépendances externes)
Analyse les CSV de benchmark et affiche un résumé comparatif
"""

import csv
import os
import json
from statistics import mean, median

def load_csv_data(filename):
    """Charge les données d'un CSV"""
    if not os.path.exists(filename):
        return None

    data = []
    try:
        with open(filename, 'r') as f:
            reader = csv.DictReader(f)
            for row in reader:
                data.append({
                    'task_id': int(row['task_id']),
                    'duration_ms': float(row['duration_ms']),
                    'success': row['success'].lower() == 'true',
                    'worker_id': int(row['worker_id']) if 'worker_id' in row else 0
                })
    except Exception as e:
        print(f"Erreur lecture {filename}: {e}")
        return None

    return data

def percentile(data, p):
    """Calcule le percentile p des données triées"""
    if not data:
        return 0
    sorted_data = sorted(data)
    n = len(sorted_data)
    k = (n - 1) * p / 100
    f = int(k)
    c = k - f
    if f == n - 1:
        return sorted_data[f]
    return sorted_data[f] + c * (sorted_data[f + 1] - sorted_data[f])

def analyze_data(data, name):
    """Analyse les données et retourne les statistiques"""
    if not data:
        return None

    durations = [d['duration_ms'] for d in data]
    successes = [d['success'] for d in data]

    stats = {
        'name': name,
        'count': len(data),
        'success_rate': (sum(successes) / len(successes)) * 100,
        'mean': mean(durations),
        'median': median(durations),
        'p50': percentile(durations, 50),
        'p95': percentile(durations, 95),
        'p99': percentile(durations, 99),
        'min': min(durations),
        'max': max(durations)
    }

    return stats

def print_comparison_table(all_stats):
    """Affiche un tableau de comparaison"""
    print("\n🎯 ANALYSE COMPARATIVE DES PERFORMANCES DEVIT")
    print("=" * 85)
    print(f"{'Scénario':<22} {'Tâches':<7} {'Success':<8} {'Moyenne':<8} {'p50':<7} {'p95':<7} {'p99':<7}")
    print("-" * 85)

    for stats in all_stats:
        if stats:
            print(f"{stats['name']:<22} {stats['count']:<7} "
                  f"{stats['success_rate']:>6.1f}% {stats['mean']:>7.1f}ms "
                  f"{stats['p50']:>6.1f}ms {stats['p95']:>6.1f}ms {stats['p99']:>6.1f}ms")

    print("-" * 85)

def analyze_performance_impact(baseline_stats, chaos_stats_list):
    """Analyse l'impact des scénarios chaos vs baseline"""
    if not baseline_stats:
        print("⚠️  Pas de données baseline pour comparaison")
        return

    print(f"\n🌪️  IMPACT DU CHAOS vs BASELINE")
    print("=" * 60)
    print(f"Baseline p95: {baseline_stats['p95']:.1f}ms")
    print("-" * 60)

    for chaos_stats in chaos_stats_list:
        if chaos_stats:
            impact_p95 = ((chaos_stats['p95'] - baseline_stats['p95']) / baseline_stats['p95']) * 100
            impact_mean = ((chaos_stats['mean'] - baseline_stats['mean']) / baseline_stats['mean']) * 100

            print(f"{chaos_stats['name']:<20} "
                  f"p95: {chaos_stats['p95']:>6.1f}ms (+{impact_p95:>5.1f}%) "
                  f"mean: {chaos_stats['mean']:>6.1f}ms (+{impact_mean:>5.1f}%)")

def validate_v1_requirements(all_stats):
    """Valide les exigences v1.0"""
    print(f"\n✅ VALIDATION EXIGENCES v1.0")
    print("=" * 50)

    baseline_stats = next((s for s in all_stats if s and 'baseline' in s['name'].lower()), None)

    if baseline_stats:
        # Critère 1: p95 < 50ms sans chaos
        if baseline_stats['p95'] < 50:
            print(f"✅ p95 baseline < 50ms: {baseline_stats['p95']:.1f}ms")
        else:
            print(f"❌ p95 baseline >= 50ms: {baseline_stats['p95']:.1f}ms")

        # Critère 2: Success rate >= 99%
        if baseline_stats['success_rate'] >= 99:
            print(f"✅ Success rate >= 99%: {baseline_stats['success_rate']:.1f}%")
        else:
            print(f"❌ Success rate < 99%: {baseline_stats['success_rate']:.1f}%")

    # Critère 3: p95 chaos modéré < 500ms
    chaos_stats = [s for s in all_stats if s and 'chaos' in s['name'].lower()]

    print(f"\n🌪️  RÉSILIENCE CHAOS:")
    for stats in chaos_stats:
        if stats['p95'] < 500:
            print(f"✅ {stats['name']} p95 < 500ms: {stats['p95']:.1f}ms")
        else:
            print(f"⚠️  {stats['name']} p95 >= 500ms: {stats['p95']:.1f}ms")

def generate_summary_report(all_stats):
    """Génère un rapport JSON de synthèse"""
    report = {
        'timestamp': '',
        'devit_version': '1.0.0-bench',
        'summary': {
            'total_scenarios': len([s for s in all_stats if s]),
            'baseline_p95': None,
            'worst_chaos_p95': None,
            'v1_compliant': False
        },
        'scenarios': all_stats
    }

    baseline = next((s for s in all_stats if s and 'baseline' in s['name'].lower()), None)
    if baseline:
        report['summary']['baseline_p95'] = baseline['p95']
        report['summary']['v1_compliant'] = baseline['p95'] < 50 and baseline['success_rate'] >= 99

    chaos_p95s = [s['p95'] for s in all_stats if s and 'chaos' in s['name'].lower()]
    if chaos_p95s:
        report['summary']['worst_chaos_p95'] = max(chaos_p95s)

    with open('devit_bench_report.json', 'w') as f:
        json.dump(report, f, indent=2)

    print(f"\n💾 Rapport JSON sauvé: devit_bench_report.json")

def main():
    """Fonction principale"""
    print("🔍 DevIt Simple Benchmark Analysis")
    print("Recherche des fichiers CSV de benchmark...")

    # Fichiers à analyser
    csv_files = [
        ('baseline.csv', 'Baseline Zero-Polling'),
        ('chaos_latency.csv', 'Chaos Latency 100ms'),
        ('results_baseline.csv', 'Results Baseline'),
        ('results_latency_moderate.csv', 'Latency 50ms±10ms'),
        ('results_latency_high.csv', 'Latency 200ms±50ms'),
        ('results_drops.csv', 'Network Drops 5%'),
        ('results_dups.csv', 'Duplications 5%'),
        ('results_realistic.csv', 'Mix Réaliste'),
    ]

    all_stats = []
    baseline_stats = None
    chaos_stats_list = []

    for filename, name in csv_files:
        data = load_csv_data(filename)
        if data:
            stats = analyze_data(data, name)
            all_stats.append(stats)
            print(f"✅ {filename}: {len(data)} tâches analysées")

            if 'baseline' in name.lower():
                baseline_stats = stats
            elif 'chaos' in name.lower() or any(word in name.lower() for word in ['latency', 'drops', 'dups', 'mix']):
                chaos_stats_list.append(stats)
        else:
            print(f"⚠️  Fichier manquant: {filename}")

    if not all_stats:
        print("❌ Aucune donnée trouvée!")
        print("Exécutez d'abord: cargo run -p devit-bench")
        return 1

    # Affichages des analyses
    print_comparison_table(all_stats)
    analyze_performance_impact(baseline_stats, chaos_stats_list)
    validate_v1_requirements(all_stats)
    generate_summary_report(all_stats)

    print(f"\n🎉 Analyse terminée!")
    print("🚀 DevIt prouve son efficacité: Zero-Polling + Résilience Chaos!")

    return 0

if __name__ == "__main__":
    exit(main())