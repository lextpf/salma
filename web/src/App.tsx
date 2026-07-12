import { Routes, Route } from 'react-router-dom'
import Layout from './comps/Layout'
import ErrorBoundary from './comps/ErrorBoundary'
import InstallPage from './InstallPage'
import LibraryPage from './LibraryPage'
import LogsPage from './LogsPage'
import SettingsPage from './SettingsPage'

function App() {
  return (
    <ErrorBoundary>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<InstallPage />} />
          <Route path="fomods" element={<LibraryPage />} />
          <Route path="fomods/:name" element={<LibraryPage />} />
          <Route path="logs" element={<LogsPage />} />
          <Route path="settings" element={<SettingsPage />} />
        </Route>
      </Routes>
    </ErrorBoundary>
  )
}

export default App
