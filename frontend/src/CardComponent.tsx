import { faCircleInfo } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group } from '@mantine/core';
import Text from '@/elements/Text.tsx';
import SafeBoundary from './SafeBoundary.tsx';
import { useExtTranslations } from './translations';

function Inner() {
  const { t: tExt } = useExtTranslations();
  return (
    <Group gap='xs' wrap='nowrap' align='flex-start'>
      <FontAwesomeIcon icon={faCircleInfo} className='text-blue-400 mt-1' />
      <Text size='xs' c='dimmed'>
        {tExt('card.description', {})}
      </Text>
    </Group>
  );
}

export default function CardComponent() {
  return (
    <SafeBoundary>
      <Inner />
    </SafeBoundary>
  );
}

export { CardComponent };
