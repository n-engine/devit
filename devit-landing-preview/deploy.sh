#!/bin/bash
# Script de déploiement pour Gandi
# Crée un package prêt à déployer

set -e

echo "📦 Préparation du déploiement pour Gandi..."

# 1. Build du site
echo "🔨 Build production..."
npm run build

# 2. Créer le répertoire de déploiement
DEPLOY_DIR="deploy-gandi"
rm -rf $DEPLOY_DIR
mkdir -p $DEPLOY_DIR

# 3. Copier les fichiers nécessaires
echo "📋 Copie des fichiers..."
cp -r dist $DEPLOY_DIR/
cp server.js $DEPLOY_DIR/
cp package.prod.json $DEPLOY_DIR/package.json

# 4. Créer une archive (optionnel)
echo "🗜️  Création de l'archive..."
tar -czf devit-landing.tar.gz -C $DEPLOY_DIR .

echo ""
echo "✅ Package de déploiement prêt !"
echo ""
echo "Fichiers dans $DEPLOY_DIR/ :"
ls -lh $DEPLOY_DIR/
echo ""
echo "Archive créée : devit-landing.tar.gz"
echo ""
echo "📤 Déploiement sur Gandi :"
echo "  1. Upload : $DEPLOY_DIR/* ou devit-landing.tar.gz"
echo "  2. Commande start : npm start"
echo "  3. Port : 8080 (ou variable PORT)"
echo ""
