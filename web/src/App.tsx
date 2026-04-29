import { Routes, Route } from 'react-router-dom'
import Layout from './components/Layout'
import ErrorBoundary from './components/ErrorBoundary'
import InstallPage from './InstallPage'
import FomodBrowserPage from './FomodBrowserPage'
import FomodDetailPage from './FomodDetailPage'
import LogsPage from './LogsPage'
import SettingsPage from './SettingsPage'

function App() {
  return (
    <ErrorBoundary>
      <Routes>
        <Route element={<Layout />}>
          <Route index element={<InstallPage />} />
          <Route path="fomods" element={<FomodBrowserPage />} />
          <Route path="fomods/:name" element={<FomodDetailPage />} />
          <Route path="logs" element={<LogsPage />} />
          <Route path="settings" element={<SettingsPage />} />
        </Route>
      </Routes>
    </ErrorBoundary>
  )
}

export default App
