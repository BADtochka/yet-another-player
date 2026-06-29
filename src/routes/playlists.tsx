import { createFileRoute } from '@tanstack/react-router';

const Playlists = () => {
  return <h1>Плейлисты</h1>;
};

export const Route = createFileRoute('/playlists')({
  component: Playlists,
});
