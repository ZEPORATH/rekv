package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/raut/rekv/clients/go/config"
	"google.golang.org/grpc/codes"
	grpcstatus "google.golang.org/grpc/status"
)

func main() {
	target := flag.String("target", "127.0.0.1:50051", "Rust configd gRPC address")
	listen := flag.String("listen", "127.0.0.1:8090", "Go demo HTTP listen address")
	webRoot := flag.String("web-root", "../../web/dist", "built React application directory")
	flag.Parse()

	client, err := config.NewClient(*target)
	if err != nil {
		log.Fatal(err)
	}
	defer client.Close()

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/config/list", listHandler(client))
	mux.HandleFunc("GET /api/config", getHandler(client))
	mux.HandleFunc("POST /api/config", setHandler(client))
	mux.HandleFunc("DELETE /api/config", deleteHandler(client))
	mux.HandleFunc("POST /api/config/restore", restoreHandler(client))
	mux.HandleFunc("GET /api/events", watchHandler(client))
	mux.Handle("/", spaHandler(*webRoot))

	server := &http.Server{
		Addr:              *listen,
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}
	log.Printf("config editor: http://%s", *listen)
	log.Fatal(server.ListenAndServe())
}

func getHandler(client *config.Client) http.HandlerFunc {
	return func(writer http.ResponseWriter, request *http.Request) {
		path, ok := requiredPath(writer, request)
		if !ok {
			return
		}
		value, err := client.Get(request.Context(), path)
		if err != nil {
			writeError(writer, err)
			return
		}
		writeJSON(writer, http.StatusOK, map[string]any{"id": 1, "ok": true, "result": value})
	}
}

func listHandler(client *config.Client) http.HandlerFunc {
	return func(writer http.ResponseWriter, request *http.Request) {
		path, ok := requiredPath(writer, request)
		if !ok {
			return
		}
		children, err := client.List(request.Context(), path)
		if err != nil {
			writeError(writer, err)
			return
		}
		writeJSON(writer, http.StatusOK, map[string]any{"id": 1, "ok": true, "result": children})
	}
}

func setHandler(client *config.Client) http.HandlerFunc {
	type setRequest struct {
		ID    uint64       `json:"id"`
		Path  string       `json:"path"`
		Value config.Value `json:"value"`
	}
	return func(writer http.ResponseWriter, request *http.Request) {
		request.Body = http.MaxBytesReader(writer, request.Body, 1<<20)
		var body setRequest
		decoder := json.NewDecoder(request.Body)
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&body); err != nil {
			writeJSON(writer, http.StatusBadRequest, map[string]any{"id": body.ID, "ok": false, "error": map[string]string{"code": "INVALID_REQUEST", "message": err.Error()}})
			return
		}
		if body.Path == "" {
			writeJSON(writer, http.StatusBadRequest, map[string]any{"id": body.ID, "ok": false, "error": map[string]string{"code": "INVALID_REQUEST", "message": "path is required"}})
			return
		}
		if err := client.Set(request.Context(), body.Path, body.Value); err != nil {
			writeError(writer, err)
			return
		}
		writeJSON(writer, http.StatusOK, map[string]any{"id": body.ID, "ok": true})
	}
}

func deleteHandler(client *config.Client) http.HandlerFunc {
	return func(writer http.ResponseWriter, request *http.Request) {
		path, ok := requiredPath(writer, request)
		if !ok {
			return
		}
		if err := client.Delete(request.Context(), path); err != nil {
			writeError(writer, err)
			return
		}
		writeJSON(writer, http.StatusOK, map[string]any{"id": 1, "ok": true})
	}
}

func restoreHandler(client *config.Client) http.HandlerFunc {
	return func(writer http.ResponseWriter, request *http.Request) {
		request.Body = http.MaxBytesReader(writer, request.Body, 64*1024)
		var body struct {
			ID   uint64 `json:"id"`
			Path string `json:"path"`
		}
		decoder := json.NewDecoder(request.Body)
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&body); err != nil || body.Path == "" {
			if err == nil {
				err = errors.New("path is required")
			}
			writeJSON(writer, http.StatusBadRequest, map[string]any{"id": body.ID, "ok": false, "error": map[string]string{"code": "INVALID_REQUEST", "message": err.Error()}})
			return
		}
		if err := client.Restore(request.Context(), body.Path); err != nil {
			writeError(writer, err)
			return
		}
		writeJSON(writer, http.StatusOK, map[string]any{"id": body.ID, "ok": true})
	}
}

func watchHandler(client *config.Client) http.HandlerFunc {
	return func(writer http.ResponseWriter, request *http.Request) {
		path, ok := requiredPath(writer, request)
		if !ok {
			return
		}
		stream, err := client.Watch(request.Context(), path)
		if err != nil {
			writeError(writer, err)
			return
		}
		flusher, ok := writer.(http.Flusher)
		if !ok {
			http.Error(writer, "streaming is unavailable", http.StatusInternalServerError)
			return
		}
		writer.Header().Set("Content-Type", "text/event-stream")
		writer.Header().Set("Cache-Control", "no-cache")
		writer.Header().Set("X-Accel-Buffering", "no")
		flusher.Flush()
		for {
			event, err := stream.Recv()
			if err != nil {
				if !errors.Is(err, context.Canceled) && err != io.EOF {
					log.Printf("watch stream ended: %v", err)
				}
				return
			}
			encoded, err := json.Marshal(event)
			if err != nil {
				return
			}
			if _, err := fmt.Fprintf(writer, "data: %s\n\n", encoded); err != nil {
				return
			}
			flusher.Flush()
		}
	}
}

func spaHandler(root string) http.Handler {
	absoluteRoot, err := filepath.Abs(root)
	if err != nil {
		log.Fatal(err)
	}
	return http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		relativePath := filepath.Clean(filepath.FromSlash(strings.TrimPrefix(request.URL.Path, "/")))
		if relativePath == ".." || strings.HasPrefix(relativePath, ".."+string(filepath.Separator)) {
			http.NotFound(writer, request)
			return
		}
		path := filepath.Join(absoluteRoot, relativePath)
		if info, err := os.Stat(path); err == nil && !info.IsDir() {
			http.ServeFile(writer, request, path)
			return
		}
		index := filepath.Join(absoluteRoot, "index.html")
		if _, err := os.Stat(index); err != nil {
			http.Error(writer, "React app is not built; run npm --prefix web run build", http.StatusServiceUnavailable)
			return
		}
		http.ServeFile(writer, request, index)
	})
}

func requiredPath(writer http.ResponseWriter, request *http.Request) (string, bool) {
	path := request.URL.Query().Get("path")
	if path == "" {
		writeJSON(writer, http.StatusBadRequest, map[string]any{"id": 0, "ok": false, "error": map[string]string{"code": "INVALID_REQUEST", "message": "path is required"}})
		return "", false
	}
	return path, true
}

func writeError(writer http.ResponseWriter, err error) {
	httpStatus := http.StatusInternalServerError
	code := "INTERNAL_ERROR"
	if rpcError, ok := err.(*config.RPCError); ok {
		code = rpcError.Code
		switch code {
		case "INVALID_REQUEST", "INVALID_PATH", "INVALID_VALUE":
			httpStatus = http.StatusBadRequest
		case "NOT_FOUND":
			httpStatus = http.StatusNotFound
		}
	} else {
		switch grpcstatus.Code(err) {
		case codes.InvalidArgument:
			httpStatus = http.StatusBadRequest
			code = "INVALID_REQUEST"
		case codes.NotFound:
			httpStatus = http.StatusNotFound
			code = "NOT_FOUND"
		case codes.Internal:
			code = "INTERNAL_ERROR"
		}
	}
	writeJSON(writer, httpStatus, map[string]any{"id": 0, "ok": false, "error": map[string]string{"code": code, "message": err.Error()}})
}

func writeJSON(writer http.ResponseWriter, status int, value any) {
	writer.Header().Set("Content-Type", "application/json")
	writer.WriteHeader(status)
	_ = json.NewEncoder(writer).Encode(value)
}
