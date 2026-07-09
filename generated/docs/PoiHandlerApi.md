# \PoiHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_poi_handler**](PoiHandlerApi.md#create_poi_handler) | **POST** /api/v1/pois | 
[**delete_poi_handler**](PoiHandlerApi.md#delete_poi_handler) | **DELETE** /api/v1/pois/{id} | 
[**get_poi_handler**](PoiHandlerApi.md#get_poi_handler) | **GET** /api/v1/pois/{id} | 
[**list_poi_handler**](PoiHandlerApi.md#list_poi_handler) | **GET** /api/v1/pois | 
[**update_poi_handler**](PoiHandlerApi.md#update_poi_handler) | **PATCH** /api/v1/pois/{id} | 



## create_poi_handler

> models::PoiExtendedModel create_poi_handler(create_poi_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_poi_schema** | [**CreatePoiSchema**](CreatePoiSchema.md) | Create point of interest (PoI) | [required] |

### Return type

[**models::PoiExtendedModel**](PoiExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_poi_handler

> delete_poi_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for deleting point of interest | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_poi_handler

> models::PoiExtendedModel get_poi_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for fetching point of interest | [required] |

### Return type

[**models::PoiExtendedModel**](PoiExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_poi_handler

> models::PagedResponsePoiExtendedModel list_poi_handler(page, limit, order_by, desc, name, user_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**name** | Option<**String**> | PoI name |  |
**user_id** | Option<**uuid::Uuid**> | user ID |  |

### Return type

[**models::PagedResponsePoiExtendedModel**](PagedResponse_PoiExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## update_poi_handler

> models::PoiExtendedModel update_poi_handler(id, update_poi_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | id for patching point of interest | [required] |
**update_poi_schema** | [**UpdatePoiSchema**](UpdatePoiSchema.md) |  | [required] |

### Return type

[**models::PoiExtendedModel**](PoiExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

