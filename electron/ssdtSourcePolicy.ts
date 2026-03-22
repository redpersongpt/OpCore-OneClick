export interface SsdtSourcePolicy {
  requestedFileName: string;
  packageCandidates: string[];
  supplementalDownload?: {
    catalog: 'dortania';
    fileName: string;
    url: string;
  };
}

const DORTANIA_ACPI_BASE =
  'https://raw.githubusercontent.com/dortania/Getting-Started-With-ACPI/master/extra-files/compiled';

export const OPENCORE_PKG_ACPI_SAMPLE_FILES = new Set([
  'SSDT-ALS0.aml',
  'SSDT-AWAC-DISABLE.aml',
  'SSDT-BRG0.aml',
  'SSDT-EC-USBX.aml',
  'SSDT-EC.aml',
  'SSDT-EHCx-DISABLE.aml',
  'SSDT-HV-DEV-WS2022.aml',
  'SSDT-HV-DEV.aml',
  'SSDT-HV-PLUG.aml',
  'SSDT-HV-VMBUS.aml',
  'SSDT-IMEI.aml',
  'SSDT-PLUG-ALT.aml',
  'SSDT-PLUG.aml',
  'SSDT-PMC.aml',
  'SSDT-PNLF.aml',
  'SSDT-RTC0-RANGE.aml',
  'SSDT-RTC0.aml',
  'SSDT-SBUS-MCHC.aml',
  'SSDT-UNC.aml',
]);

export const DORTANIA_COMPILED_ACPI_FILES = new Set([
  'SSDT-AWAC.aml',
  'SSDT-CPUR.aml',
  'SSDT-EC-DESKTOP.aml',
  'SSDT-EC-LAPTOP.aml',
  'SSDT-EC-USBX-DESKTOP.aml',
  'SSDT-EC-USBX-LAPTOP.aml',
  'SSDT-IMEI-S.aml',
  'SSDT-IMEI.aml',
  'SSDT-PLUG-DRTNIA.aml',
  'SSDT-PMC.aml',
  'SSDT-PNLF.aml',
  'SSDT-RHUB.aml',
  'SSDT-RTC0-RANGE-HEDT.aml',
  'SSDT-UNC.aml',
  'SSDT-XOSI.aml',
]);

function dortaniaSupplemental(fileName: string): NonNullable<SsdtSourcePolicy['supplementalDownload']> {
  return {
    catalog: 'dortania',
    fileName,
    url: `${DORTANIA_ACPI_BASE}/${fileName}`,
  };
}

const EXPLICIT_SOURCE_POLICIES: Record<string, SsdtSourcePolicy> = {
  'SSDT-AWAC.aml': {
    requestedFileName: 'SSDT-AWAC.aml',
    packageCandidates: ['SSDT-AWAC.aml', 'SSDT-AWAC-DISABLE.aml'],
    supplementalDownload: dortaniaSupplemental('SSDT-AWAC.aml'),
  },
  'SSDT-CPUR.aml': {
    requestedFileName: 'SSDT-CPUR.aml',
    packageCandidates: ['SSDT-CPUR.aml'],
    supplementalDownload: dortaniaSupplemental('SSDT-CPUR.aml'),
  },
  'SSDT-EC-USBX-DESKTOP.aml': {
    requestedFileName: 'SSDT-EC-USBX-DESKTOP.aml',
    packageCandidates: ['SSDT-EC-USBX-DESKTOP.aml'],
    supplementalDownload: dortaniaSupplemental('SSDT-EC-USBX-DESKTOP.aml'),
  },
  'SSDT-XOSI.aml': {
    requestedFileName: 'SSDT-XOSI.aml',
    packageCandidates: ['SSDT-XOSI.aml'],
    supplementalDownload: dortaniaSupplemental('SSDT-XOSI.aml'),
  },
};

export function getSsdtSourcePolicy(requestedFileName: string): SsdtSourcePolicy | null {
  const explicit = EXPLICIT_SOURCE_POLICIES[requestedFileName];
  if (explicit) return explicit;

  if (OPENCORE_PKG_ACPI_SAMPLE_FILES.has(requestedFileName)) {
    return {
      requestedFileName,
      packageCandidates: [requestedFileName],
    };
  }

  if (DORTANIA_COMPILED_ACPI_FILES.has(requestedFileName)) {
    return {
      requestedFileName,
      packageCandidates: [requestedFileName],
      supplementalDownload: dortaniaSupplemental(requestedFileName),
    };
  }

  return null;
}

export function getUnsupportedSsdtRequests(requestedFileNames: string[]): string[] {
  return Array.from(new Set(requestedFileNames.filter((name) => getSsdtSourcePolicy(name) === null))).sort();
}
