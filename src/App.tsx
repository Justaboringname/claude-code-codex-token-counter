import { PopoverContent } from './PopoverContent';
import './App.css';

function App() {
  const browserPreview = typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);
  return (
    <div
      style={{
        background: browserPreview
          ? 'radial-gradient(120% 140% at 18% -8%, #8EC5F5 0%, #5B8FE8 32%, #7A68D6 58%, #35306E 88%, #1E1B45 100%)'
          : 'transparent',
      }}
    >
      <PopoverContent />
    </div>
  );
}

export default App;
