const fs = require('fs');
const path = require('path');

const rootDir = path.resolve(__dirname, '..');
const pkg = require(path.join(rootDir, 'package.json'));
const currentVersion = pkg.version;
const outDir = path.join(rootDir, 'build-output', 'RL-Tracker-Suite-Windows');
const targetRelease = path.join(rootDir, 'target', 'release');
const nsisDir = path.join(targetRelease, 'bundle', 'nsis');

if (!fs.existsSync(outDir)) {
  fs.mkdirSync(outDir, { recursive: true });
}

// Nettoyer les anciens exécutables dans build-output
for (const file of fs.readdirSync(outDir)) {
  if (file.endsWith('.exe')) {
    try {
      fs.unlinkSync(path.join(outDir, file));
    } catch (e) {
      // Ignorer si verrouillé temporairement
    }
  }
}

// Copier le binaire portable
const portableSrc = path.join(targetRelease, 'rl-tracker-suite.exe');
if (fs.existsSync(portableSrc)) {
  const portableDest = path.join(outDir, 'RL Tracker Suite (portable).exe');
  try {
    fs.copyFileSync(portableSrc, portableDest);
    console.log(`Copie portable : ${portableDest}`);
  } catch (err) {
    if (err.code === 'EBUSY') {
      console.warn(`[AVERTISSEMENT] ${portableDest} est en cours d'utilisation par Windows. L'exécutable release a été généré dans src-tauri/target/release.`);
    } else {
      throw err;
    }
  }
}

// Copier uniquement l'installeur NSIS correspondant à la version courante
if (fs.existsSync(nsisDir)) {
  const files = fs.readdirSync(nsisDir).filter((f) => f.endsWith('.exe'));
  let targetFile = files.find((f) => f.includes(`_${currentVersion}_`));
  if (!targetFile && files.length > 0) {
    targetFile = files
      .map((f) => ({ name: f, time: fs.statSync(path.join(nsisDir, f)).mtimeMs }))
      .sort((a, b) => b.time - a.time)[0].name;
  }

  if (targetFile) {
    const src = path.join(nsisDir, targetFile);
    const dest = path.join(outDir, targetFile);
    fs.copyFileSync(src, dest);
    console.log(`Copie setup NSIS (version courante ${currentVersion}) : ${dest}`);
  }
}

console.log(`Fichiers de build synchronises avec succes dans build-output/RL-Tracker-Suite-Windows !`);
