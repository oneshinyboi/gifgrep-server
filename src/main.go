package main

import (
	"crypto/subtle"
	"database/sql"
	"encoding/json"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
	"time"

	_ "modernc.org/sqlite"
)

const schema = `
CREATE TABLE IF NOT EXISTS favorites (
  id         TEXT PRIMARY KEY,
  url        TEXT NOT NULL,
  preview    TEXT,
  provider   TEXT,
  title      TEXT,
  added_at   INTEGER NOT NULL,
  last_used  INTEGER,
  use_count  INTEGER NOT NULL DEFAULT 0
);
`

type favorite struct {
	ID       string  `json:"id"`
	URL      string  `json:"url"`
	Preview  string  `json:"preview"`
	Provider string  `json:"provider"`
	Title    string  `json:"title"`
	UseCount int     `json:"use_count"`
	AddedAt  string  `json:"added_at"`
	LastUsed *string `json:"last_used"`
}

type server struct {
	db    *sql.DB
	token string
}

func newServer(db *sql.DB, token string) *server {
	return &server{db: db, token: token}
}

func iso(secs int64) string {
	return time.Unix(secs, 0).UTC().Format(time.RFC3339)
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, map[string]string{"error": msg})
}

func (s *server) authorized(r *http.Request) bool {
	if s.token == "" {
		return false
	}
	got := r.Header.Get("X-Auth-Token")
	return len(got) == len(s.token) && subtle.ConstantTimeCompare([]byte(got), []byte(s.token)) == 1
}

func (s *server) fetch(db *sql.DB, id string) (*favorite, error) {
	row := db.QueryRow(`SELECT id, url, preview, provider, title, added_at, last_used, use_count FROM favorites WHERE id = ?`, id)
	var f favorite
	var preview, provider, title sql.NullString
	var addedAt int64
	var lastUsed sql.NullInt64
	err := row.Scan(&f.ID, &f.URL, &preview, &provider, &title, &addedAt, &lastUsed, &f.UseCount)
	if err != nil {
		return nil, err
	}
	f.Preview = preview.String
	f.Provider = provider.String
	f.Title = title.String
	f.AddedAt = iso(addedAt)
	if lastUsed.Valid {
		lu := iso(lastUsed.Int64)
		f.LastUsed = &lu
	}
	return &f, nil
}

func (s *server) health(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func (s *server) listFavorites(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		writeErr(w, http.StatusUnauthorized, "missing or invalid X-Auth-Token")
		return
	}
	limit := 0
	if q := r.URL.Query().Get("limit"); q != "" {
		n, err := strconv.Atoi(q)
		if err != nil || n < 0 {
			writeErr(w, http.StatusBadRequest, "invalid limit")
			return
		}
		limit = n
	}
	q := `SELECT id FROM favorites ORDER BY use_count DESC, last_used DESC, id ASC`
	if limit > 0 {
		q += ` LIMIT ` + strconv.Itoa(limit)
	}
	ids, err := s.db.Query(q)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	defer ids.Close()
	var idList []string
	for ids.Next() {
		var id string
		if err := ids.Scan(&id); err != nil {
			writeErr(w, http.StatusInternalServerError, "db error")
			return
		}
		idList = append(idList, id)
	}
	if err := ids.Err(); err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	items := make([]favorite, 0, len(idList))
	for _, id := range idList {
		f, err := s.fetch(s.db, id)
		if err != nil {
			writeErr(w, http.StatusInternalServerError, "db error")
			return
		}
		items = append(items, *f)
	}
	writeJSON(w, http.StatusOK, items)
}

func (s *server) postFavorite(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		writeErr(w, http.StatusUnauthorized, "missing or invalid X-Auth-Token")
		return
	}
	var body struct {
		ID       string `json:"id"`
		URL      string `json:"url"`
		Preview  string `json:"preview"`
		Provider string `json:"provider"`
		Title    string `json:"title"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		writeErr(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	id := strings.TrimSpace(body.ID)
	url := strings.TrimSpace(body.URL)
	if id == "" || url == "" {
		writeErr(w, http.StatusBadRequest, "id and url are required")
		return
	}
	if !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") {
		writeErr(w, http.StatusBadRequest, "url must start with http:// or https://")
		return
	}
	now := time.Now().Unix()
	res, err := s.db.Exec(`
		INSERT INTO favorites (id, url, preview, provider, title, added_at, last_used, use_count)
		VALUES (?, ?, ?, ?, ?, ?, NULL, 0)
		ON CONFLICT(id) DO UPDATE SET
			url = excluded.url,
			preview = excluded.preview,
			provider = excluded.provider,
			title = excluded.title`,
		id, url, body.Preview, body.Provider, body.Title, now)
	_ = res
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	f, err := s.fetch(s.db, id)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	writeJSON(w, http.StatusOK, f)
}

func (s *server) useFavorite(w http.ResponseWriter, r *http.Request, id string) {
	if !s.authorized(r) {
		writeErr(w, http.StatusUnauthorized, "missing or invalid X-Auth-Token")
		return
	}
	id = strings.TrimSpace(id)
	now := time.Now().Unix()
	res, err := s.db.Exec(`UPDATE favorites SET use_count = use_count + 1, last_used = ? WHERE id = ?`, now, id)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	n, _ := res.RowsAffected()
	if n == 0 {
		writeErr(w, http.StatusNotFound, "favorite not found")
		return
	}
	f, err := s.fetch(s.db, id)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"id":        id,
		"use_count": f.UseCount,
		"last_used": f.LastUsed,
	})
}

func (s *server) deleteFavorite(w http.ResponseWriter, r *http.Request, id string) {
	if !s.authorized(r) {
		writeErr(w, http.StatusUnauthorized, "missing or invalid X-Auth-Token")
		return
	}
	id = strings.TrimSpace(id)
	res, err := s.db.Exec(`DELETE FROM favorites WHERE id = ?`, id)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "db error")
		return
	}
	n, _ := res.RowsAffected()
	if n == 0 {
		writeErr(w, http.StatusNotFound, "favorite not found")
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusNoContent)
}

func (s *server) serveMux() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", s.health)
	mux.HandleFunc("/api/v1/favorites", s.favoritesRoot)
	mux.HandleFunc("/api/v1/favorites/", s.favoritesItem)
	return logging(mux)
}

func (s *server) favoritesRoot(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		s.listFavorites(w, r)
	case http.MethodPost:
		s.postFavorite(w, r)
	default:
		writeErr(w, http.StatusMethodNotAllowed, "method not allowed")
	}
}

func (s *server) favoritesItem(w http.ResponseWriter, r *http.Request) {
	rest := strings.TrimPrefix(r.URL.Path, "/api/v1/favorites/")
	if strings.HasSuffix(rest, "/use") {
		id := strings.TrimSuffix(rest, "/use")
		if r.Method != http.MethodPatch {
			writeErr(w, http.StatusMethodNotAllowed, "method not allowed")
			return
		}
		s.useFavorite(w, r, id)
		return
	}
	if r.Method != http.MethodDelete {
		writeErr(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}
	s.deleteFavorite(w, r, rest)
}

func logging(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		lw := &statusWriter{ResponseWriter: w, status: 200}
		next.ServeHTTP(lw, r)
		log.Printf("%s %s %d", r.Method, r.URL.Path, lw.status)
	})
}

type statusWriter struct {
	http.ResponseWriter
	status int
}

func (l *statusWriter) WriteHeader(code int) {
	l.status = code
	l.ResponseWriter.WriteHeader(code)
}

func openDB(path string) (*sql.DB, error) {
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, err
	}
	if _, err := db.Exec(`PRAGMA journal_mode=WAL;`); err != nil {
		db.Close()
		return nil, err
	}
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, err
	}
	db.SetMaxOpenConns(1)
	return db, nil
}

func main() {
	token := os.Getenv("FAV_TOKEN")
	if token == "" {
		_, _ = os.Stderr.WriteString("gif-favs: FAV_TOKEN is required; refusing to start\n")
		os.Exit(1)
	}
	port := os.Getenv("FAV_PORT")
	if port == "" {
		port = "8099"
	}
	host := os.Getenv("FAV_HOST")
	if host == "" {
		host = "0.0.0.0"
	}
	dbPath := os.Getenv("FAV_DB_PATH")
	if dbPath == "" {
		dbPath = "./favorites.db"
	}
	db, err := openDB(dbPath)
	if err != nil {
		log.Fatalf("gif-favs: open db: %v", err)
	}
	defer db.Close()

	s := newServer(db, token)
	addr := net.JoinHostPort(host, port)
	srv := &http.Server{Addr: addr, Handler: s.serveMux()}

	go func() {
		log.Printf("gif-favs: listening on %s (db=%s)", addr, dbPath)
		if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Fatalf("gif-favs: server: %v", err)
		}
	}()

	stopsig := make(chan os.Signal, 1)
	signal.Notify(stopsig, syscall.SIGINT, syscall.SIGTERM)
	<-stopsig
	log.Printf("gif-favs: shutting down")
	done := make(chan struct{}, 1)
	go func() {
		srv.Close()
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		_, _ = os.Stderr.WriteString("gif-favs: forced exit\n")
		os.Exit(1)
	}
}
