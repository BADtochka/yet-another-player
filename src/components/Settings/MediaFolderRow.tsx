import { Folder } from '@/components/Icons/Folder';
import { SettingsTextAction } from '@/components/Settings/SettingsTextAction';

type MediaFolderRowProps = {
  path: string;
  onRemove: () => void;
};

export const MediaFolderRow = ({ path, onRemove }: MediaFolderRowProps) => {
  return (
    <div className='flex items-center gap-2.5 self-stretch p-2.5'>
      <Folder size={24} className='shrink-0 text-[#506ba9]' aria-hidden />
      <p className='min-w-0 truncate text-base font-medium text-[#d9d9d9]' title={path}>
        {path}
      </p>
      <SettingsTextAction onClick={onRemove} className='ml-auto'>
        Удалить
      </SettingsTextAction>
    </div>
  );
};
