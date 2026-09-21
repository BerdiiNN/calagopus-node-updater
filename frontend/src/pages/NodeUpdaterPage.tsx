import { faClockRotateLeft, faGear, faKey, faServer } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Tabs } from '@mantine/core';
import { useState } from 'react';
import AdminContentContainer from '@/elements/containers/AdminContentContainer.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import OverviewTab from '../components/OverviewTab.tsx';
import ProfilesTab from '../components/ProfilesTab.tsx';
import RunsTab from '../components/RunsTab.tsx';
import SettingsTab from '../components/SettingsTab.tsx';
import SafeBoundary from '../SafeBoundary.tsx';
import { useExtTranslations } from '../translations';

export type TabId = 'overview' | 'runs' | 'profiles' | 'settings';

function Inner() {
  const { t: tExt } = useExtTranslations();
  const canSettings = useAdminCan('node-updater.settings');
  const [tab, setTab] = useState<TabId>('overview');

  return (
    <Tabs value={tab} onChange={(value) => setTab((value as TabId) ?? 'overview')} keepMounted={false}>
      <Tabs.List mb='md'>
        <Tabs.Tab value='overview' leftSection={<FontAwesomeIcon icon={faServer} />}>
          {tExt('tabs.overview', {})}
        </Tabs.Tab>
        <Tabs.Tab value='runs' leftSection={<FontAwesomeIcon icon={faClockRotateLeft} />}>
          {tExt('tabs.runs', {})}
        </Tabs.Tab>
        {canSettings && (
          <Tabs.Tab value='profiles' leftSection={<FontAwesomeIcon icon={faKey} />}>
            {tExt('tabs.profiles', {})}
          </Tabs.Tab>
        )}
        {canSettings && (
          <Tabs.Tab value='settings' leftSection={<FontAwesomeIcon icon={faGear} />}>
            {tExt('tabs.settings', {})}
          </Tabs.Tab>
        )}
      </Tabs.List>

      <Tabs.Panel value='overview'>
        <OverviewTab onShowRuns={() => setTab('runs')} />
      </Tabs.Panel>
      <Tabs.Panel value='runs'>
        <RunsTab />
      </Tabs.Panel>
      {canSettings && (
        <Tabs.Panel value='profiles'>
          <ProfilesTab />
        </Tabs.Panel>
      )}
      {canSettings && (
        <Tabs.Panel value='settings'>
          <SettingsTab />
        </Tabs.Panel>
      )}
    </Tabs>
  );
}

export default function NodeUpdaterPage() {
  const { t: tExt } = useExtTranslations();
  return (
    <SafeBoundary>
      <AdminContentContainer title={tExt('common.nodeUpdater', {})} subtitle={tExt('common.subtitle', {})}>
        <Inner />
      </AdminContentContainer>
    </SafeBoundary>
  );
}

export { NodeUpdaterPage };
