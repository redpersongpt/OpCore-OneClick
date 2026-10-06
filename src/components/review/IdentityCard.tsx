import { useState } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import type { PlatformIdentity } from '../../bridge/types';
import { useT } from '../../i18n';
import { maskSecret } from '../../lib/format';
import { Button } from '../ui/Button';
import { KeyValue, Section } from '../ui/Section';

/** SMBIOS identity; serial-like values are masked until the user reveals them. */
export function IdentityCard({ identity, secureBootModel }: { identity: PlatformIdentity; secureBootModel: string }) {
  const t = useT();
  const [revealed, setRevealed] = useState(false);
  const show = (value: string) => (revealed ? value : maskSecret(value, 3));

  return (
    <Section
      title={t('review.identity')}
      description={t('review.identityHint')}
      actions={
        <Button size="sm" variant="ghost" icon={revealed ? <EyeOff /> : <Eye />} onClick={() => setRevealed(!revealed)}>
          {revealed ? t('review.hide') : t('review.show')}
        </Button>
      }
    >
      <KeyValue label={t('review.model')}>{identity.model}</KeyValue>
      <KeyValue label={t('review.serial')} mono>
        {show(identity.serial)}
      </KeyValue>
      <KeyValue label={t('review.mlb')} mono>
        {show(identity.mlb)}
      </KeyValue>
      <KeyValue label={t('review.uuid')} mono>
        {show(identity.systemUuid)}
      </KeyValue>
      <KeyValue label={t('review.rom')} mono>
        {show(identity.rom)}
      </KeyValue>
      <KeyValue label={t('plan.secureBoot')} mono>
        {secureBootModel}
      </KeyValue>
    </Section>
  );
}
