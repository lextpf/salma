import { Routes, Route } from 'react-router-dom'
import Layout from './components/Layout'
import ErrorBoundary from './components/ErrorBoundary'
import InstallPage from './pages/InstallPage'
import FomodBrowserPage from './pages/FomodBrowserPage'
import FomodDetailPage from './pages/FomodDetailPage'
import LogsPage from './pages/LogsPage'
import SettingsPage from './pages/SettingsPage'

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
