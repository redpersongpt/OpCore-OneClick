# OpCore-OneClick

<div align="center">

<a href="https://macos-install.one/">
  <img src="./image.png" alt="OpCore-OneClick" width="100%">
</a>

<br/>

[![Website](https://img.shields.io/badge/Website-macos--install.one-000000?style=for-the-badge&logo=apple&logoColor=white)](https://macos-install.one/)
[![GitHub Stars](https://img.shields.io/github/stars/redpersongpt/OpCore-OneClick?style=for-the-badge&logo=github&color=gold)](https://github.com/redpersongpt/OpCore-OneClick/stargazers)
[![License](https://img.shields.io/github/license/redpersongpt/OpCore-OneClick?style=for-the-badge&logo=apache&color=brightgreen)](LICENSE)
[![Follow on X](https://img.shields.io/badge/Follow-%40redpersongpt-000000?style=for-the-badge&logo=x&logoColor=white)](https://x.com/redpersongpt)

![Windows](https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)
![macOS](https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)

</div>

---

Review-first desktop utility for building and validating OpenCore-based macOS installer workflows on supported PC hardware.

OpCore-OneClick, formerly macOS One-Click, scans the target machine, validates the hardware path, builds the EFI, fetches required recovery resources where applicable, and prepares deployment media without hiding the risky parts.

## Overview

- Hardware scan for CPU, GPU, chipset, storage, and peripherals
- Compatibility validation before EFI generation or deployment
- OpenCore EFI build for the detected hardware path
- macOS recovery download from Apple infrastructure where applicable
- USB or partition deployment with resumable downloads

## Workflow

1. Scan the machine and identify the real hardware path.
2. Validate display, SMBIOS, and EFI constraints before write.
3. Build the OpenCore configuration and supporting files.
4. Fetch the required recovery and dependency assets.
5. Deploy to a USB drive or boot partition.

## Safeguards

- Blocks unsupported GPU and display paths early
- Verifies EFI layout and configuration consistency before deployment
- Keeps recovery downloads resumable across restarts when the saved session is still valid
- Leaves BIOS changes and final install decisions under user review

## Download

**[Download the latest release](https://github.com/redpersongpt/OpCore-OneClick/releases/latest)**

## Acknowledgements

Built on the work of [Acidanthera](https://github.com/acidanthera) and [Dortania](https://dortania.github.io/OpenCore-Install-Guide/).

---

<div align="center">

**[macos-install.one](https://macos-install.one/)** &nbsp;·&nbsp; **[redpersongpt](https://github.com/redpersongpt)** &nbsp;·&nbsp; [![Follow on X](https://img.shields.io/badge/-%40redpersongpt-000?logo=x&logoColor=white)](https://x.com/redpersongpt)

[![Star History Chart](https://api.star-history.com/svg?repos=redpersongpt/OpCore-OneClick&type=Date)](https://star-history.com/#redpersongpt/OpCore-OneClick&Date)

</div>
