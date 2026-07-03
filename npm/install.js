#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

// 1. Mapeo del sistema actual a los nombres que usamos en GitHub Actions
const platformToOs = {
  win32: 'windows',
  darwin: 'macos',
  linux: 'linux'
};

const archToArch = {
  x64: 'amd64',
  // Si luego en GitHub Actions agregas compilación para Mac M1/M2, agregarías: arm64: 'aarch64'
};

const os = platformToOs[process.platform];
const arch = archToArch[process.arch];

if (!os || !arch) {
  console.error(`❌ Error: Tu sistema operativo (${process.platform}) o arquitectura (${process.arch}) no están soportados por LazyCF todavía.`);
  process.exit(1);
}

// 2. Configuración del repositorio
// IMPORTANTE: Cambia "tu-usuario" por tu nombre de usuario real en GitHub
const REPO = 'PaulPPS632/lazycf'; 

// NPM inyecta la versión del package.json en esta variable de entorno. 
// Si falla, cae al '0.1.0' por defecto.
const VERSION = process.env.npm_package_version || '0.1.0'; 

const isWindows = os === 'windows';
const assetName = isWindows ? `lazycf-windows-amd64.exe` : `lazycf-${os}-amd64`;
const finalBinName = isWindows ? 'lazycf.exe' : 'lazycf';

// Construir la URL de descarga (Asumiendo que tus tags en GitHub empiezan con 'v', ej: v0.1.0)
const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${assetName}`;

// 3. Preparar la carpeta de destino
const binDir = path.join(__dirname, 'bin');
const binPath = path.join(binDir, finalBinName);

async function download() {
  console.log(`📥 Descargando LazyCF v${VERSION} para ${os}-${arch}...`);
  console.log(`🔗 URL: ${url}`);

  try {
    // Crear la carpeta bin/ si no existe
    if (!fs.existsSync(binDir)) {
      fs.mkdirSync(binDir, { recursive: true });
    }

    // Hacer la petición HTTP
    const response = await fetch(url);

    if (!response.ok) {
      throw new Error(`Error HTTP: ${response.status} - ${response.statusText}`);
    }

    // Transformar la respuesta a un Buffer y escribir el archivo
    const arrayBuffer = await response.arrayBuffer();
    const buffer = Buffer.from(arrayBuffer);
    fs.writeFileSync(binPath, buffer);

    // 4. Dar permisos de ejecución (Crucial para Linux y macOS)
    if (!isWindows) {
      fs.chmodSync(binPath, 0o755); // Otorga permisos rwxr-xr-x
    }

    console.log('✅ ¡LazyCF se instaló correctamente! Escribe "lazycf" en tu terminal para empezar.');
  } catch (error) {
    console.error('\n❌ Error al descargar el binario de LazyCF:');
    console.error(error.message);
    console.error('\n💡 Verifica que publicaste el Release en GitHub con la etiqueta v' + VERSION + ' y que los archivos se llamen exactamente como se espera.');
    process.exit(1);
  }
}

download();