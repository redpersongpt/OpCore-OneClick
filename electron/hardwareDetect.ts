import { exec } from 'child_process';
import util from 'util';
import os from 'os';
import fs from 'fs';
import { inferLaptopFormFactor } from './formFactor.js';

const execPromise = util.promisify(exec);

// ── Types ─────────────────────────────────────────────────────────────────────

export type DetectionConfidence = 'detected' | 'partially-detected' | 'unverified';

export interface GpuDevice {
  name: string;
  vendorId: string | null;   // e.g. "10de", "1002", "8086"
  deviceId: string | null;   // e.g. "2484"
  vendorName: string;        // resolved from vendorId or raw name
  confidence: DetectionConfidence;
}

export interface CpuInfo {
  name: string;
  vendor: string;            // "GenuineIntel" | "AuthenticAMD" | "Apple" | ...
  vendorName: string;        // "Intel" | "AMD" | "Apple"
  confidence: DetectionConfidence;
}

export interface DetectedHardware {
  cpu: CpuInfo;
  gpus: GpuDevice[];
  primaryGpu: GpuDevice;
  motherboardVendor: string;
  motherboardModel: string;
  ramBytes: number;
  coreCount: number;
  isLaptop: boolean;
  isVM: boolean;
}

// ── PCI vendor ID map (GPU vendors relevant to Hackintosh) ────────────────────

const GPU_VENDOR_MAP: Record<string, string> = {
  '10de': 'NVIDIA',
  '1002': 'AMD',
  '8086': 'Intel',
  '1414': 'Microsoft',  // Hyper-V virtual GPU
  '15ad': 'VMware',
  '1234': 'QEMU',
};

export function resolveGpuVendor(vendorId: string | null, rawName: string): string {
  if (vendorId) {
    const known = GPU_VENDOR_MAP[vendorId.toLowerCase()];
    if (known) return known;
  }
  const n = rawName.toLowerCase();
  if (n.includes('microsoft remote display adapter') || n.includes('microsoft basic display adapter') || n.includes('remote display adapter')) return 'Microsoft';
  if (n.includes('nvidia') || n.includes('geforce') || n.includes('quadro') || n.includes('rtx') || n.includes('gtx')) return 'NVIDIA';
  if (n.includes('amd') || n.includes('radeon') || n.includes('rx ') || n.includes('vega') || n.includes('navi')) return 'AMD';
  if (n.includes('intel') || n.includes('iris') || n.includes('uhd') || n.includes('hd graphics')) return 'Intel';
  return 'Unknown';
}

function pickFallbackCpuName(): string {
  return os.cpus()[0]?.model?.trim() || 'Unknown CPU';
}

function isSoftwareDisplayAdapter(gpu: GpuDevice): boolean {
  const name = gpu.name.toLowerCase();
  return gpu.vendorId === '1414'
    || name.includes('remote display adapter')
    || name.includes('basic display adapter')
    || name.includes('render only')
    || name.includes('indirect display');
}

export function normalizeWindowsGpuList(gpus: GpuDevice[]): GpuDevice[] {
  const filtered = gpus.filter((gpu) => !isSoftwareDisplayAdapter(gpu));
  if (filtered.length > 0) return filtered;
  return gpus;
}

function resolveCpuVendor(vendorStr: string, rawName: string): { vendor: string; vendorName: string } {
  const v = vendorStr.toLowerCase();
  if (v.includes('genuineintel') || v.includes('intel')) return { vendor: 'GenuineIntel', vendorName: 'Intel' };
  if (v.includes('authenticamd') || v.includes('amd')) return { vendor: 'AuthenticAMD', vendorName: 'AMD' };
  if (v.includes('apple')) return { vendor: 'Apple', vendorName: 'Apple' };
  // Fallback from CPU name
  const n = rawName.toLowerCase();
  if (n.includes('intel')) return { vendor: 'GenuineIntel', vendorName: 'Intel' };
  if (n.includes('amd') || n.includes('ryzen') || n.includes('threadripper')) return { vendor: 'AuthenticAMD', vendorName: 'AMD' };
  if (n.includes('apple') || n.includes(' m1') || n.includes(' m2') || n.includes(' m3') || n.includes(' m4')) return { vendor: 'Apple', vendorName: 'Apple' };
  return { vendor: vendorStr || 'Unknown', vendorName: 'Unknown' };
}

export const WINDOWS_HARDWARE_QUERIES = {
  cpuName: '(Get-CimInstance Win32_Processor).Name',
  cpuVendor: '(Get-CimInstance Win32_Processor).Manufacturer',
  gpuJson: 'Get-CimInstance Win32_VideoController | Select-Object Name, PNPDeviceID | ConvertTo-Json -Compress',
  boardJson: 'Get-CimInstance Win32_BaseBoard | Select-Object Manufacturer, Product | ConvertTo-Json -Compress',
  chassisTypes: '(Get-CimInstance Win32_SystemEnclosure).ChassisTypes',
  manufacturer: '(Get-CimInstance Win32_ComputerSystem).Manufacturer',
  model: '(Get-CimInstance Win32_ComputerSystem).Model',
  batteryJson: 'Get-CimInstance Win32_Battery | Select-Object -First 1 | ConvertTo-Json -Compress',
  coreCount: '(Get-CimInstance Win32_Processor).NumberOfCores',
} as const;

// ── Windows ───────────────────────────────────────────────────────────────────

export async function detectWindowsHardware(): Promise<DetectedHardware> {
  const ps = (cmd: string, fallback = '') =>
    execPromise(`powershell -NoProfile -Command "${cmd}"`, {
      timeout: 3_500,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRes, cpuVendorRes, gpuRes, boardRes, chassisRes, manufRes, modelRes, batteryRes, coresRes] = await Promise.all([
    ps(WINDOWS_HARDWARE_QUERIES.cpuName, 'Unknown CPU'),
    ps(WINDOWS_HARDWARE_QUERIES.cpuVendor),
    // PNPDeviceID gives "PCI\\VEN_10DE&DEV_2484&..." — extract vendor+device IDs
    ps(WINDOWS_HARDWARE_QUERIES.gpuJson),
    ps(WINDOWS_HARDWARE_QUERIES.boardJson),
    ps(WINDOWS_HARDWARE_QUERIES.chassisTypes),
    ps(WINDOWS_HARDWARE_QUERIES.manufacturer),
    ps(WINDOWS_HARDWARE_QUERIES.model),
    ps(WINDOWS_HARDWARE_QUERIES.batteryJson),
    ps(WINDOWS_HARDWARE_QUERIES.coreCount),
  ]);

  const cpuName = cpuRes.stdout.trim().split('\n')[0] || pickFallbackCpuName();
  const cpuVendorRaw = cpuVendorRes.stdout.trim() || '';
  const { vendor, vendorName } = resolveCpuVendor(cpuVendorRaw, cpuName);

  // Parse GPU JSON (may be array or single object)
  let gpus: GpuDevice[] = [];
  try {
    const raw = JSON.parse(gpuRes.stdout.trim());
    const entries = Array.isArray(raw) ? raw : [raw];
    gpus = entries.filter(Boolean).map((e: any) => {
      const pnp: string = e.PNPDeviceID ?? '';
      const venMatch = pnp.match(/VEN_([0-9A-Fa-f]{4})/);
      const devMatch = pnp.match(/DEV_([0-9A-Fa-f]{4})/);
      const vendorId = venMatch ? venMatch[1].toLowerCase() : null;
      const deviceId = devMatch ? devMatch[1].toLowerCase() : null;
      const name: string = e.Name ?? 'Unknown GPU';
      return {
        name,
        vendorId,
        deviceId,
        vendorName: resolveGpuVendor(vendorId, name),
        confidence: vendorId ? 'detected' : 'partially-detected',
      } satisfies GpuDevice;
    });
  } catch {
    const rawName = gpuRes.stdout.trim().split('\n')[0] || 'Unknown GPU';
    gpus = [{ name: rawName, vendorId: null, deviceId: null, vendorName: resolveGpuVendor(null, rawName), confidence: 'partially-detected' }];
  }
  if (gpus.length === 0) {
    gpus = [{ name: 'Unknown GPU', vendorId: null, deviceId: null, vendorName: 'Unknown', confidence: 'unverified' }];
  }
  gpus = normalizeWindowsGpuList(gpus);

  // Board
  let boardVendor = 'Unknown', boardModel = 'Unknown';
  try {
    const b = JSON.parse(boardRes.stdout.trim());
    const board = Array.isArray(b) ? b[0] : b;
    boardVendor = board?.Manufacturer ?? 'Unknown';
    boardModel = board?.Product ?? 'Unknown';
  } catch {}

  // Chassis / laptop detection
  const chassisNums = (chassisRes.stdout.match(/\d+/g) ?? []).map(Number);
  const isLaptop = inferLaptopFormFactor({
    cpuName,
    chassisTypes: chassisNums,
    modelName: modelRes.stdout.trim(),
    batteryPresent: batteryRes.stdout.trim().length > 0 && batteryRes.stdout.trim() !== 'null',
  });

  // VM detection
  const manuf = manufRes.stdout.trim().toLowerCase();
  const isVM = /vmware|qemu|innotek|microsoft corporation|parallels|xen|hyper-v/i.test(manuf);

  const coreCount = parseInt(coresRes.stdout.trim()) || os.cpus().length;

  return {
    cpu: { name: cpuName, vendor, vendorName, confidence: vendor !== 'Unknown' ? 'detected' : 'unverified' },
    gpus,
    primaryGpu: gpus[0],
    motherboardVendor: boardVendor,
    motherboardModel: boardModel,
    ramBytes: os.totalmem(),
    coreCount,
    isLaptop,
    isVM,
  };
}

// ── Linux ─────────────────────────────────────────────────────────────────────

export async function detectLinuxHardware(): Promise<DetectedHardware> {
  const run = (cmd: string, fallback = '') =>
    execPromise(cmd, {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRes, gpuRes, boardVendorRes, boardModelRes, chassisRes, sysVendorRes, batteryRes, memRes] = await Promise.all([
    run('cat /proc/cpuinfo'),
    run('lspci -nn 2>/dev/null | grep -iE "VGA|3D|Display"'),
    run('cat /sys/class/dmi/id/board_vendor 2>/dev/null'),
    run('cat /sys/class/dmi/id/board_name 2>/dev/null || cat /sys/class/dmi/id/product_name 2>/dev/null'),
    run('cat /sys/class/dmi/id/chassis_type 2>/dev/null'),
    run('cat /sys/class/dmi/id/sys_vendor 2>/dev/null'),
    run('ls /sys/class/power_supply 2>/dev/null | grep -E "^BAT"'),
    run('grep MemTotal /proc/meminfo'),
  ]);

  // CPU
  const cpuLines = cpuRes.stdout.split('\n');
  const cpuName = (cpuLines.find(l => l.startsWith('model name')) ?? '').split(':')[1]?.trim() || 'Unknown CPU';
  const cpuVendorRaw = (cpuLines.find(l => l.startsWith('vendor_id')) ?? '').split(':')[1]?.trim() || '';
  const { vendor, vendorName } = resolveCpuVendor(cpuVendorRaw, cpuName);

  // GPU — `lspci -nn` format: "01:00.0 VGA compatible controller [0300]: NVIDIA ... [10de:2484] (rev a1)"
  const gpuLines = gpuRes.stdout.trim().split('\n').filter(Boolean);
  const gpus: GpuDevice[] = gpuLines.map(line => {
    const idMatch = line.match(/\[([0-9a-f]{4}):([0-9a-f]{4})\]/i);
    const vendorId = idMatch ? idMatch[1].toLowerCase() : null;
    const deviceId = idMatch ? idMatch[2].toLowerCase() : null;
    // Name is between "]: " and " [xx:xx]" or end
    const nameMatch = line.match(/\]:\s*(.+?)(?:\s*\[[0-9a-f]{4}:[0-9a-f]{4}\])?(?:\s*\(rev|$)/i);
    const name = nameMatch ? nameMatch[1].trim() : line.split(':').slice(2).join(':').trim() || 'Unknown GPU';
    return { name, vendorId, deviceId, vendorName: resolveGpuVendor(vendorId, name), confidence: vendorId ? 'detected' as const : 'partially-detected' as const };
  });
  if (gpus.length === 0) gpus.push({ name: 'Unknown GPU', vendorId: null, deviceId: null, vendorName: 'Unknown', confidence: 'unverified' });

  // RAM
  const memKB = parseInt((memRes.stdout.match(/\d+/) ?? ['0'])[0]);
  const ramBytes = memKB * 1024 || os.totalmem();

  // Chassis / laptop
  const chassisType = parseInt(chassisRes.stdout.trim()) || 0;
  const isLaptop = inferLaptopFormFactor({
    cpuName,
    chassisTypes: chassisType ? [chassisType] : [],
    modelName: boardModelRes.stdout.trim(),
    batteryPresent: batteryRes.stdout.trim().length > 0,
  });

  // VM
  const sysVendor = sysVendorRes.stdout.trim().toLowerCase();
  const isVM = /vmware|qemu|innotek|microsoft|parallels|xen/i.test(sysVendor);

  return {
    cpu: { name: cpuName, vendor, vendorName, confidence: vendor !== 'Unknown' ? 'detected' : 'unverified' },
    gpus,
    primaryGpu: gpus[0],
    motherboardVendor: boardVendorRes.stdout.trim() || 'Unknown',
    motherboardModel: boardModelRes.stdout.trim() || 'Unknown',
    ramBytes,
    coreCount: os.cpus().length,
    isLaptop,
    isVM,
  };
}

// ── macOS ─────────────────────────────────────────────────────────────────────

export async function detectMacHardware(): Promise<DetectedHardware> {
  const run = (cmd: string, fallback = '') =>
    execPromise(cmd, {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    }).catch(() => ({ stdout: fallback }));

  const [cpuRes, cpuVendorRes, gpuRes, memRes, modelRes] = await Promise.all([
    run('sysctl -n machdep.cpu.brand_string'),
    run('sysctl -n machdep.cpu.vendor 2>/dev/null'),
    run("system_profiler SPDisplaysDataType 2>/dev/null | grep -E 'Chipset Model|Vendor ID|Device ID'"),
    run('sysctl -n hw.memsize'),
    run('system_profiler SPHardwareDataType 2>/dev/null | grep -E "Model Identifier|Model Name"'),
  ]);

  const cpuName = cpuRes.stdout.trim() || 'Unknown CPU';
  const cpuVendorRaw = cpuVendorRes.stdout.trim();
  const { vendor, vendorName } = resolveCpuVendor(cpuVendorRaw, cpuName);

  // Parse GPU from system_profiler output
  // Lines like: "      Chipset Model: AMD Radeon RX 6600 XT"
  //             "      Vendor ID: 0x1002"
  //             "      Device ID: 0x73ff"
  const gpuLines = gpuRes.stdout.split('\n').map(l => l.trim());
  const gpus: GpuDevice[] = [];
  let currentName = '', currentVendorId: string | null = null, currentDeviceId: string | null = null;

  for (const line of gpuLines) {
    if (line.startsWith('Chipset Model:')) {
      if (currentName) {
        gpus.push({ name: currentName, vendorId: currentVendorId, deviceId: currentDeviceId, vendorName: resolveGpuVendor(currentVendorId, currentName), confidence: currentVendorId ? 'detected' : 'partially-detected' });
      }
      currentName = line.split(':')[1]?.trim() || 'Unknown GPU';
      currentVendorId = null; currentDeviceId = null;
    } else if (line.startsWith('Vendor ID:')) {
      currentVendorId = line.split(':')[1]?.trim().replace('0x', '').toLowerCase() || null;
    } else if (line.startsWith('Device ID:')) {
      currentDeviceId = line.split(':')[1]?.trim().replace('0x', '').toLowerCase() || null;
    }
  }
  if (currentName) {
    gpus.push({ name: currentName, vendorId: currentVendorId, deviceId: currentDeviceId, vendorName: resolveGpuVendor(currentVendorId, currentName), confidence: currentVendorId ? 'detected' : 'partially-detected' });
  }
  if (gpus.length === 0) gpus.push({ name: 'Unknown GPU', vendorId: null, deviceId: null, vendorName: 'Unknown', confidence: 'unverified' });

  const ramBytes = parseInt(memRes.stdout.trim()) || os.totalmem();
  const modelLines = modelRes.stdout.split('\n').map(l => l.trim());
  const modelId = (modelLines.find(l => l.startsWith('Model Identifier')) ?? '').split(':')[1]?.trim() || '';
  const modelName = (modelLines.find(l => l.startsWith('Model Name')) ?? '').split(':')[1]?.trim() || 'Unknown Mac';
  const isLaptop = modelId.toLowerCase().includes('book') || modelName.toLowerCase().includes('book');

  return {
    cpu: { name: cpuName, vendor, vendorName, confidence: vendor !== 'Unknown' ? 'detected' : 'unverified' },
    gpus,
    primaryGpu: gpus[0],
    motherboardVendor: 'Apple',
    motherboardModel: modelId || modelName,
    ramBytes,
    coreCount: os.cpus().length,
    isLaptop,
    isVM: false,
  };
}

import { sim } from './simulation.js';

// ── Entry point ───────────────────────────────────────────────────────────────

export async function detectHardware(): Promise<DetectedHardware> {
  if (sim.isEnabled('hw:timeout')) {
    await new Promise(r => setTimeout(r, 15000));
  }
  if (sim.isEnabled('hw:invalid-data')) {
    throw new Error('[SIMULATED_FAILURE] Hardware detection returned corrupt descriptors');
  }

  if (process.platform === 'win32') return detectWindowsHardware();
  if (process.platform === 'linux')  return detectLinuxHardware();
  return detectMacHardware();
}
