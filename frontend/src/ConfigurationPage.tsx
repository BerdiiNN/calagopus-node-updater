import { faArrowUpRightFromSquare } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Stack } from '@mantine/core';
import Button from '@/elements/Button.tsx';
import Text from '@/elements/Text.tsx';
import SafeBoundary from './SafeBoundary.tsx';
import SettingsTab from './components/SettingsTab.tsx';
import { navigateTo } from './lib/navigate.ts';
import { useExtTranslations } from './translations';

function Inner() {
  const { t: tExt } = useExtTranslations();
  return (
    <Stack gap='md'>
      <Group justify='space-between' wrap='wrap'>
        <Text size='sm' c='dimmed' className='max-w-3xl'>
          {tExt('card.description', {})}
        </Text>
        <Button
          size='compact-sm'
          variant='default'
          leftSection={<FontAwesomeIcon icon={faArrowUpRightFromSquare} />}
          onClick={() => navigateTo('/admin/node-updater')}
        >
          {tExt('card.open', {})}
        </Button>
      </Group>
      <SettingsTab />
    </Stack>
  );
}

export default function ConfigurationPage() {
  return (
    <SafeBoundary>
      <Inner />
    </SafeBoundary>
  );
}

export { ConfigurationPage };
