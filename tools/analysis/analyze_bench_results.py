#!/usr/bin/env python3
"""
DevIt Benchmark Analysis Script
Analyse les résultats CSV et génère des graphiques comparatifs
pour prouver l'efficacité du système zero-polling DevIt
"""

import pandas as pd
import matplotlib.pyplot as plt
import numpy as np
import os
import sys

def load_csv_if_exists(filename):
    """Charge un CSV s'il existe, sinon retourne None"""
    if os.path.exists(filename):
        return pd.read_csv(filename)
    return None

def analyze_csv(df, name):
    """Analyse un DataFrame et retourne les statistiques"""
    if df is None:
        return None

    durations = df['duration_ms']
    success_rate = df['success'].mean() * 100

    stats = {
        'name': name,
        'count': len(df),
        'success_rate': success_rate,
        'mean': durations.mean(),
        'p50': durations.quantile(0.5),
        'p95': durations.quantile(0.95),
        'p99': durations.quantile(0.99),
        'min': durations.min(),
        'max': durations.max(),
    }

    return stats

def create_comparison_chart(datasets):
    """Crée un graphique comparatif des latences"""
    fig, ((ax1, ax2), (ax3, ax4)) = plt.subplots(2, 2, figsize=(15, 10))
    fig.suptitle('🚀 DevIt Performance Analysis - Zero Polling vs Chaos', fontsize=16, fontweight='bold')

    # Graphique 1: Distribution des latences
    ax1.set_title('Distribution des latences (histogramme)')
    ax1.set_xlabel('Latence (ms)')
    ax1.set_ylabel('Nombre de tâches')

    colors = ['#2E8B57', '#FF6B35', '#4CAF50', '#FF9800', '#9C27B0']

    for i, (df, stats) in enumerate(datasets):
        if df is not None:
            color = colors[i % len(colors)]
            ax1.hist(df['duration_ms'], bins=30, alpha=0.6,
                    label=f"{stats['name']} (p95={stats['p95']:.1f}ms)",
                    color=color)

    ax1.legend()
    ax1.grid(True, alpha=0.3)

    # Graphique 2: Box plot comparatif
    ax2.set_title('Comparaison des percentiles')
    ax2.set_ylabel('Latence (ms)')

    box_data = []
    labels = []

    for df, stats in datasets:
        if df is not None:
            box_data.append(df['duration_ms'])
            labels.append(stats['name'])

    if box_data:
        ax2.boxplot(box_data, labels=labels)
        ax2.tick_params(axis='x', rotation=45)

    # Graphique 3: Barres p95
    ax3.set_title('Comparaison p95 (critère validation v1.0)')
    ax3.set_ylabel('p95 Latence (ms)')
    ax3.axhline(y=50, color='red', linestyle='--', label='Limite v1.0 (50ms)')
    ax3.axhline(y=500, color='orange', linestyle='--', label='Limite chaos (500ms)')

    names = []
    p95_values = []

    for df, stats in datasets:
        if df is not None:
            names.append(stats['name'])
            p95_values.append(stats['p95'])

    if names:
        bars = ax3.bar(names, p95_values, color=colors[:len(names)])
        ax3.set_ylim(0, max(p95_values) * 1.2 if p95_values else 100)

        # Annotations sur les barres
        for bar, p95 in zip(bars, p95_values):
            height = bar.get_height()
            ax3.annotate(f'{p95:.1f}ms',
                        xy=(bar.get_x() + bar.get_width() / 2, height),
                        xytext=(0, 3),  # 3 points vertical offset
                        textcoords="offset points",
                        ha='center', va='bottom', fontweight='bold')

    ax3.tick_params(axis='x', rotation=45)
    ax3.legend()
    ax3.grid(True, alpha=0.3)

    # Graphique 4: Success rate
    ax4.set_title('Taux de succès (%)')
    ax4.set_ylabel('Success Rate (%)')
    ax4.axhline(y=99, color='red', linestyle='--', label='Objectif v1.0 (99%)')

    success_rates = []
    for df, stats in datasets:
        if df is not None:
            success_rates.append(stats['success_rate'])

    if names:
        bars = ax4.bar(names, success_rates, color=colors[:len(names)])
        ax4.set_ylim(95, 100.5)

        # Annotations
        for bar, rate in zip(bars, success_rates):
            height = bar.get_height()
            ax4.annotate(f'{rate:.1f}%',
                        xy=(bar.get_x() + bar.get_width() / 2, height),
                        xytext=(0, 3),
                        textcoords="offset points",
                        ha='center', va='bottom', fontweight='bold')

    ax4.tick_params(axis='x', rotation=45)
    ax4.legend()
    ax4.grid(True, alpha=0.3)

    plt.tight_layout()
    return fig

def print_summary_table(all_stats):
    """Affiche un tableau récapitulatif"""
    print("\n🎯 RÉCAPITULATIF DES PERFORMANCES")
    print("=" * 80)
    print(f"{'Scénario':<20} {'Tâches':<8} {'Success%':<9} {'Moy.':<8} {'p50':<8} {'p95':<8} {'p99':<8}")
    print("-" * 80)

    for stats in all_stats:
        if stats:
            print(f"{stats['name']:<20} {stats['count']:<8} "
                  f"{stats['success_rate']:<8.1f}% {stats['mean']:<7.1f}ms "
                  f"{stats['p50']:<7.1f}ms {stats['p95']:<7.1f}ms {stats['p99']:<7.1f}ms")

    print("-" * 80)

    # Validation v1.0
    print("\n✅ VALIDATION v1.0:")
    baseline_stats = next((s for s in all_stats if s and 'baseline' in s['name'].lower()), None)

    if baseline_stats:
        if baseline_stats['p95'] < 50:
            print(f"✅ Baseline p95 < 50ms: {baseline_stats['p95']:.1f}ms - EXCELLENT")
        else:
            print(f"❌ Baseline p95 >= 50ms: {baseline_stats['p95']:.1f}ms")

        if baseline_stats['success_rate'] >= 99:
            print(f"✅ Success rate >= 99%: {baseline_stats['success_rate']:.1f}%")
        else:
            print(f"❌ Success rate < 99%: {baseline_stats['success_rate']:.1f}%")

    # Analyse chaos
    chaos_stats = [s for s in all_stats if s and 'chaos' in s['name'].lower()]
    if chaos_stats:
        print(f"\n🌪️  RÉSILIENCE CHAOS:")
        for stats in chaos_stats:
            if stats['p95'] < 500:
                print(f"✅ {stats['name']} p95 < 500ms: {stats['p95']:.1f}ms")
            else:
                print(f"⚠️  {stats['name']} p95 >= 500ms: {stats['p95']:.1f}ms")

def main():
    """Fonction principale d'analyse"""
    print("🔍 DevIt Benchmark Analysis")
    print("Recherche des fichiers CSV...")

    # Fichiers à analyser (ordre d'affichage)
    csv_files = [
        ('baseline.csv', 'Baseline'),
        ('chaos_latency.csv', 'Chaos Latency'),
        ('results_baseline.csv', 'Results Baseline'),
        ('results_latency_moderate.csv', 'Latency Moderate'),
        ('results_latency_high.csv', 'Latency High'),
        ('results_drops.csv', 'Network Drops'),
        ('results_dups.csv', 'Duplications'),
        ('results_realistic.csv', 'Realistic Mix'),
    ]

    datasets = []
    all_stats = []

    for filename, name in csv_files:
        df = load_csv_if_exists(filename)
        if df is not None:
            stats = analyze_csv(df, name)
            datasets.append((df, stats))
            all_stats.append(stats)
            print(f"✅ Chargé: {filename} ({len(df)} tâches)")
        else:
            print(f"⚠️  Manquant: {filename}")

    if not datasets:
        print("❌ Aucun fichier CSV trouvé!")
        print("Exécutez d'abord les benchmarks avec devit-bench")
        return 1

    # Affichage du tableau récapitulatif
    print_summary_table(all_stats)

    # Génération du graphique
    print("\n📊 Génération des graphiques...")
    fig = create_comparison_chart(datasets)

    # Sauvegarde
    output_file = 'devit_benchmark_analysis.png'
    fig.savefig(output_file, dpi=300, bbox_inches='tight')
    print(f"💾 Graphique sauvé: {output_file}")

    # Affichage si possible
    try:
        plt.show()
    except:
        print("Note: plt.show() non disponible, graphique sauvé seulement")

    print("\n🎉 Analyse terminée!")
    print(f"Résultats: {output_file}")

    return 0

if __name__ == "__main__":
    sys.exit(main())