import { SwitchSettings } from '@/components/Settings/Compressor';
import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/settings')({
  component: RouteComponent,
});

function RouteComponent() {
  return (
    <div className='flex flex-col max-w-244 w-full mx-auto'>
      <SwitchSettings label='Test'></SwitchSettings>
    </div>
  );
}
