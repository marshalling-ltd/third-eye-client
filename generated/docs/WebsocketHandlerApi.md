# \WebsocketHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_ws_connection**](WebsocketHandlerApi.md#get_ws_connection) | **GET** /api/v1/ws | 
[**ws_connect**](WebsocketHandlerApi.md#ws_connect) | **GET** /api/v1/ws/{token} | 



## get_ws_connection

> models::WebsocketResponse get_ws_connection()


### Parameters

This endpoint does not need any parameter.

### Return type

[**models::WebsocketResponse**](WebsocketResponse.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/stream, application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## ws_connect

> ws_connect(token)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**token** | **String** | Websocket connection token (see GET /api/v1/ws) | [required] |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/stream, application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

