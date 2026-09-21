import { faCloudArrowDown } from '@fortawesome/free-solid-svg-icons';
import { type FC, lazy } from 'react';
import { Extension, ExtensionContext } from 'shared';
import CardComponent from './CardComponent.tsx';
import ConfigurationPage from './ConfigurationPage.tsx';
import UpdateNotice from './components/UpdateNotice.tsx';
import UpdatesHealthCard from './components/UpdatesHealthCard.tsx';
import { NavigateBridge } from './lib/navigate.ts';
import { getExtTranslations } from './translations';

const NodeUpdaterPage = lazy(() => import('./pages/NodeUpdaterPage.tsx'));

export const NODE_UPDATER_PATH = '/node-updater';

class NodeUpdaterExtension extends Extension {
  public packageName = 'com.pineriver.nodeupdater';

  public cardComponent: FC | null = CardComponent;
  public cardConfigurationPage: FC | null = ConfigurationPage;

  public initialize(ctx: ExtensionContext): void {
    ctx.extensionRegistry.enterRoutes((routes) => {
      routes.addAdminRoute({
        name: () => getExtTranslations().t('common.nodeUpdater', {}),
        icon: faCloudArrowDown,
        path: NODE_UPDATER_PATH,
        element: NodeUpdaterPage,
        permission: ['node-updater.read'],
        category: 'infrastructure',
      });
    });

    // The global slot renders OUTSIDE the panel's AuthProvider (and before login), so only
    // the router bridge lives there. Anything that needs the user or an API call is mounted
    // through page containers inside the auth tree below.
    ctx.extensionRegistry.enterGlobal((global) => {
      global.appendComponent(NavigateBridge);
    });

    ctx.extensionRegistry.enterPages((pages) => {
      // Once-per-session "updates available" toast for administrators: the dashboard is the
      // first page after login (whichever grouping the user prefers), admin home as well.
      pages.enterDashboard((dashboard) =>
        dashboard.enterHome((home) => {
          home.containerAll.prependComponent(UpdateNotice);
          home.containerGrouped.prependComponent(UpdateNotice);
        }),
      );
      pages.enterAdmin((admin) =>
        admin.enterHome((home) => {
          home.container.prependComponent(UpdateNotice);
          // Card on Admin → Overview → Health with the fleet's update state.
          home.health.cards.appendComponent(UpdatesHealthCard);
        }),
      );
    });
  }
}

export default new NodeUpdaterExtension();
